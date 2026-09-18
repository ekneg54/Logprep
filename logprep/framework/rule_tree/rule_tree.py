"""RuleTree - thin wrapper around the Rust PyRuleTree.

The Rust core works with ``FilterExpressionInner`` and u64 rule ids.
This wrapper maps rule ids back to the Python ``Rule`` objects.
"""

from logging import getLogger
from typing import TYPE_CHECKING

from logprep._rust import PyRuleTree  # pylint: disable=no-name-in-module
from logprep.util.helper import deduplicate_with_order

if TYPE_CHECKING:
    from logprep.processor.base.rule import Rule

logger = getLogger("RuleTree")


class RuleTree:
    """Rule tree that maps between Python Rule objects and Rust rule ids."""

    def __init__(self, config: str | None = None):
        self._rule_id_to_rule: dict[int, "Rule"] = {}
        self._rule_to_id: dict[int, int] = {}
        self._inner = PyRuleTree()
        self._next_rule_id = 0
        self.tree_config = RuleTree.Config() if config is None else self._load_config(config)

    class Config:
        """Rule tree configuration (priority and tag mappings)."""

        def __init__(self, priority_dict: dict | None = None, tag_map: dict | None = None):
            self.priority_dict = priority_dict or {}
            self.tag_map = tag_map or {}

    def _load_config(self, config_path: str) -> "Config":
        """Load the rule tree configuration from a path or URL."""
        from logprep.util import getter  # pylint: disable=import-outside-toplevel

        config_data = getter.GetterFactory.from_string(config_path).get_dict()
        return RuleTree.Config(**config_data)

    @property
    def number_of_rules(self) -> int:
        """Return the number of rules in the tree."""
        return len(self._rule_id_to_rule)

    @property
    def inner(self) -> "PyRuleTree":
        """Return the underlying Rust rule tree."""
        return self._inner

    @property
    def rule_id_to_rule(self) -> dict[int, "Rule"]:
        """Return the live rule-id to rule-object mapping of the wrapper."""
        return self._rule_id_to_rule

    def add_rule(self, rule: "Rule"):
        """Add a rule to the rule tree."""
        try:
            segments = self._inner.parse_rule(
                rule.filter,
                self.tree_config.priority_dict,
                self.tree_config.tag_map,
            )
        except Exception as error:  # pylint: disable=broad-exception-caught
            logger.warning(
                'Error parsing rule "%s.yml": %s: %s. Ignore and continue.',
                getattr(rule, "file_name", None),
                type(error).__name__,
                error,
            )
            return

        rule_id = self._next_rule_id
        self._next_rule_id += 1

        self._inner.add_rule(rule_id, segments)

        self._rule_id_to_rule[rule_id] = rule
        self._rule_to_id[id(rule)] = rule_id

    def get_matching_rules(self, event: dict) -> list["Rule"]:
        """Return all matching rules, mapped back to rule objects."""
        rule_ids = self._inner.get_matching_rules(event)
        return deduplicate_with_order(
            [self._rule_id_to_rule[rid] for rid in rule_ids if rid in self._rule_id_to_rule]
        )

    def get_rule_id(self, rule: "Rule") -> int | None:
        """Return the rule id for a given rule object, if present."""
        return self._rule_to_id.get(id(rule))

    @property
    def rules(self) -> list["Rule"]:
        """Return all rules in insertion order."""
        return list(self._rule_id_to_rule.values())

    @property
    def root(self):
        """Return the tree root (legacy compatibility, always None)."""
        return None

    def print(self, *_):
        """Print the tree (legacy compatibility, no-op)."""

    def get_size(self) -> int:
        """Return the number of nodes in the tree."""
        return self._inner.size()
