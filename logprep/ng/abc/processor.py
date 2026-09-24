"""Abstract module for processors"""

# pylint: disable=import-error,no-name-in-module
# `logprep._rust.processor` is registered at runtime by the Rust extension;
# pylint cannot resolve the submodule statically.

import inspect
import logging
import typing
from collections.abc import Sequence
from typing import Any, ClassVar

from attrs import define, field, validators

from logprep._rust.processor import PyProcessorCore
from logprep.framework.rule_tree.rule_tree import RuleTree
from logprep.ng.abc.component import NgComponent as Component
from logprep.ng.abc.event import LogEvent
from logprep.processor.base.exceptions import ProcessingWarning
from logprep.processor.base.rule import Rule
from logprep.util.environ import ENV_VARS
from logprep.util.helper import (
    FieldValue,
    add_and_overwrite,
    add_fields_to,
    get_dotted_field_value,
    has_dotted_field,
)
from logprep.util.rule_loader import RuleLoader


@define
class OutputSpec:
    """
    Specifies an output by name and which target (e.g. topic for kafka, index for opensearch)
    should be addressed.
    """

    output_name: str = field(validator=(validators.instance_of(str), validators.min_len(1)))
    output_target: str = field(validator=(validators.instance_of(str), validators.min_len(1)))


logger = logging.getLogger("Processor")


class Processor(Component):
    """Abstract Processor Class to define the Interface"""

    @define(kw_only=True, slots=False)
    class Config(Component.Config):
        """Common Configurations"""

        rules: list[str] = field(
            validator=[
                validators.instance_of(list),
                validators.deep_iterable(member_validator=validators.instance_of((str, dict))),
            ]
        )
        """List of rule locations to load rules from.
        In addition to paths to file directories it is possible to retrieve rules from a URI.
        For valid URI formats see :ref:`getters`.
        As last option it is possible to define entire rules with all their configuration parameters
        as list elements.
        """
        tree_config: str | None = field(
            default=None, validator=validators.instance_of((str, type(None)))
        )
        """Path to a JSON file with a valid :ref:`Rule Tree Configuration`.
        For string format see :ref:`getters`."""
        apply_multiple_times: bool = field(default=False, validator=validators.instance_of(bool))
        """Set if the processor should be applied multiple times. This enables further processing
        of an output with the same processor."""

    __slots__ = [
        "_event",
        "_core",
        "_bypass_rule_tree",
    ]

    rule_class: ClassVar[type[Rule] | None] = None
    spec_config_keys: ClassVar[frozenset[str]] = frozenset()
    """Configuration keys that are forwarded to the Rust ``RuleSpec`` factory.
    Only processors whose rule application is migrated to Rust (Phase 4) set
    this; empty for the Python-callback processors."""
    _event: LogEvent
    _core: PyProcessorCore
    _strategy = None
    _bypass_rule_tree: bool

    def __init__(self, name: str, configuration: "Processor.Config") -> None:
        super().__init__(name, configuration)
        self._bypass_rule_tree = False
        if ENV_VARS.get("LOGPREP_BYPASS_RULE_TREE"):
            self._bypass_rule_tree = True
            logger.debug("Bypassing rule tree for processor %s", self.name)
        self._core = PyProcessorCore(
            apply_multiple_times=self.config.apply_multiple_times,
            bypass_rule_tree=self._bypass_rule_tree,
        )
        self._rule_tree = RuleTree(config=self.config.tree_config)
        self.load_rules(rules_targets=self.config.rules)

    @property
    def _rule_tree(self) -> RuleTree:
        return self.__dict__["_rule_tree"]

    @_rule_tree.setter
    def _rule_tree(self, tree: RuleTree) -> None:
        """Bind the rule tree to the Rust processor core.

        The core holds references to the live ``_inner`` and ``_rule_id_to_rule``
        attributes of the wrapper, so later ``RuleTree.add_rule(...)`` calls or
        tree replacements take effect without an explicit resync. Rules that are
        added afterwards are additionally routed to ``_register_spec`` so that
        migrated (Phase 4) processors register their Rust ``RuleSpec`` regardless
        of whether the rules arrive via ``load_rules`` or an external
        ``RuleTree.add_rule`` call (e.g. in tests).

        Parameters
        ----------
        tree : RuleTree
            The rule tree wrapper to bind.
        """
        self.__dict__["_rule_tree"] = tree
        tree.on_rule_added = self._register_spec
        self._core.set_tree(tree.inner, tree.rule_id_to_rule)

    @property
    def config(self) -> Config:
        """Provides the properly typed configuration object"""
        return typing.cast("Processor.Config", self._config)

    @property
    def rules(self) -> Sequence["Rule"]:
        """Returns all rules

        Returns
        -------
        rules: Sequence[Rule]
        """
        return self._rule_tree.rules

    @property
    def metric_labels(self) -> dict:
        """Return metric labels."""
        return {
            "component": "processor",
            "description": self.description,
            "type": self.config.type,
            "name": self.name,
        }

    async def has_asyncio(self) -> bool:
        """Return whether the processor performs asynchronous I/O operations."""

        return False

    async def process(self, event: LogEvent) -> LogEvent:
        """Process a log event.

        Parameters
        ----------
        event : dict
           A dictionary representing a log event.

        Returns
        -------
        LogEvent
            A LogEvent object containing the processed event, errors, warnings and
            extra data

        """
        # TODO make processors async
        self._event = event
        logger.debug("%s processing event %s", self.description, event)
        outcome = self._core.process(event.data, self._apply_rule_in_python)
        # `matched_rule_ids` is the only channel for rule metrics (documented
        # contract in the migration plan): Python updates the counters solely
        # via these IDs.
        rule_id_to_rule = self._rule_tree.rule_id_to_rule
        for rule_id in outcome.matched_rule_ids:
            rule = rule_id_to_rule.get(rule_id)
            if rule is not None:
                rule.metrics.number_of_processed_events += 1
        event.warnings.extend(outcome.warnings)
        for error in outcome.errors:
            event.mark_failed(error)
        return self._event

    def _apply_rule_in_python(self, rule_id: int, event: dict) -> None:
        """Apply a single matched rule via the Python callback.

        Called by the Rust core once per matched rule id. `_apply_rules` is
        declared `async`, but processors must not actually suspend: the core
        invokes this hook synchronously, so the coroutine is driven to
        completion here. Exceptions raised by the rule propagate to the core,
        which classifies them into warnings and errors.
        """
        rule = self._rule_tree.rule_id_to_rule[rule_id]
        result = self._apply_rules(event, rule)
        if not inspect.iscoroutine(result):
            return
        try:
            result.send(None)
        except StopIteration:
            return
        result.close()
        raise RuntimeError(
            f"{self.name}: _apply_rules suspended on await, "
            "but the Rust processor core only supports synchronous rule execution"
        )

    def _register_spec(self, rule_id: int, rule: Rule) -> None:
        """Register the Rust ``RuleSpec`` for one rule (Phase 4).

        Called by the ``RuleTree`` (see ``on_rule_added``) for every rule that
        is added to the tree. Only processors that have been migrated set
        ``_spec_factory`` (before ``Processor.__init__``) and ``spec_config_keys``;
        for the remaining Python-callback processors this is a no-op.

        Parameters
        ----------
        rule_id : int
            The rule id assigned by the rule tree.
        rule : Rule
            The Python rule object whose config is forwarded to the spec factory.
        """
        factory = getattr(self, "_spec_factory", None)
        if factory is None:
            return
        rule_config = rule._config  # pylint: disable=protected-access
        rule_data = {
            key: getattr(rule_config, key)
            for key in type(self).spec_config_keys
            if hasattr(rule_config, key)
        }
        for key, value in list(rule_data.items()):
            if isinstance(value, set):
                rule_data[key] = sorted(value)
        factory.make_and_register(
            core=self._core,
            rule_id=rule_id,
            rule_data=rule_data,
        )
        # Opt-in Python-Bridge fuer Prozessoren, deren per-Event-Logik Rust
        # nicht abbilden kann (z.B. dynamische URIs beim generic_adder): Die
        # Rule selbst kann die Bridgemethode `_spec_python_bridge` definieren;
        # alle anderen Regeln registrieren keinen Bridge.
        bridge = getattr(rule, "_spec_python_bridge", None)
        if bridge is not None:
            self._core.set_rule_spec_bridge(rule_id, bridge)

    async def _apply_rules(self, event: dict, rule: Rule) -> None:  # pragma: no cover
        """Default rule application hook.

        Non-migrated processors override this with their event-processing logic.
        Migrated (Phase 4) processors must not define it — their rules are applied
        by the Rust ``RuleSpec`` on the core; reaching this default means a rule
        has matched whose spec was not registered.
        """
        raise NotImplementedError(
            f"{self.name}: rule application is handled by the Rust RuleSpec "
            "on the ProcessorCore; no Python _apply_rules is available for "
            f"rule id {getattr(rule, 'id', None)}"
        )

    def test_rules(self) -> dict | None:
        """Perform custom rule tests.

        Returns a dict with a list of test results as tuples containing a result and an expected
        result for each rule, i.e. {'RULE REPR': [('Result string', 'Expected string')]}
        Optional: Can be used in addition to regular rule tests.

        """

    def load_rules(self, rules_targets: Sequence[str | dict]) -> None:
        """method to add rules from directories or urls"""
        try:
            rules = RuleLoader(rules_targets, self.name).rules
        except ValueError as error:
            logger.error("Loading rules from %s failed: %s ", rules_targets, error)
            raise error
        for rule in rules:
            self._rule_tree.add_rule(rule)
        if logger.isEnabledFor(logging.DEBUG):
            number_rules = self._rule_tree.number_of_rules
            logger.debug("%s loaded %s rules", self.description, number_rules)

    @staticmethod
    def _field_exists(event: dict, dotted_field: str) -> bool:
        return has_dotted_field(event, dotted_field)

    def _handle_warning_error(
        self,
        event: dict[str, FieldValue],
        rule: Rule,
        error: Exception,
        failure_tags: list[str] | None = None,
    ) -> None:
        tags = get_dotted_field_value(event, "tags")
        if failure_tags is None:
            failure_tags = rule.failure_tags
        if tags is None:
            new_field = {"tags": sorted(list({*failure_tags}))}
        else:
            tags_list = tags if isinstance(tags, list) else [tags]
            new_field = {"tags": sorted(list({*tags_list, *failure_tags}))}
        add_and_overwrite(event, new_field, rule)
        if isinstance(error, ProcessingWarning):
            if error.tags:
                tags = tags if tags is not None else []
                tags_list = tags if isinstance(tags, list) else [tags]
                new_field = {"tags": sorted(list({*error.tags, *tags_list, *failure_tags}))}
                add_and_overwrite(event, new_field, rule)
            self._event.warnings.append(error)
        else:
            self._event.warnings.append(ProcessingWarning(str(error), rule, event))

    def _has_missing_values(self, event: dict, rule: "Rule", source_field_dict: dict) -> bool:
        missing_fields = list(
            dict(filter(lambda x: x[1] in [None, ""], source_field_dict.items())).keys()
        )
        if missing_fields:
            if getattr(rule, "ignore_missing_fields", False):
                return True
            error = Exception(f"{self.name}: no value for fields: {missing_fields}")
            self._handle_warning_error(event, rule, error)
            return True
        return False

    def _write_target_field(self, event: dict, rule: "Rule", result: Any) -> None:
        if hasattr(rule, "target_field"):
            add_fields_to(
                event,
                fields={getattr(rule, "target_field"): result},
                merge_with_target=getattr(rule, "merge_with_target", False),
                overwrite_target=getattr(rule, "overwrite_target", False),
            )

    async def setup(self) -> None:
        """Set up the processor."""

        await super().setup()

        for rule in self.rules:
            _ = rule.metrics  # initialize metrics to show them on startup
