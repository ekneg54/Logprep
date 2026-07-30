"""RuleTree — thin wrapper around Rust PyRuleTree.

Der Rust-Core arbeitet mit FilterExpressionInner + u64 Rule-IDs.
Dieser Wrapper mapped Rule-IDs auf Python Rule-Objekte.
"""

from logging import getLogger
from typing import TYPE_CHECKING

from logprep._rust import PyRuleTree
from logprep.filter.expression import FilterExpression
from logprep.util.helper import deduplicate_with_order

if TYPE_CHECKING:
    from logprep.processor.base.rule import Rule

logger = getLogger("RuleTree")


class RuleTree:
    """Rule tree that maps between Python Rule objects and Rust rule IDs."""

    def __init__(self, config: str | None = None):
        self._rule_id_to_rule: dict[int, "Rule"] = {}
        self._rule_to_id: dict[int, int] = {}
        self._inner = PyRuleTree()
        self._next_rule_id = 0
        self.tree_config = RuleTree.Config() if config is None else self._load_config(config)

    class Config:
        def __init__(self, priority_dict: dict | None = None, tag_map: dict | None = None):
            self.priority_dict = priority_dict or {}
            self.tag_map = tag_map or {}

    def _load_config(self, config_path: str) -> "Config":
        from logprep.util import getter

        config_data = getter.GetterFactory.from_string(config_path).get_dict()
        return RuleTree.Config(**config_data)

    @property
    def number_of_rules(self) -> int:
        return len(self._rule_id_to_rule)

    def add_rule(self, rule: "Rule"):
        """Fügt eine Rule in den RuleTree ein."""
        try:
            segments = self._inner.parse_rule(
                rule.filter,
                self.tree_config.priority_dict,
                self.tree_config.tag_map,
            )
        except Exception as error:
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
        """Holt alle Rule-IDs die matchen, mapped zurück zu Rule-Objekten."""
        rule_ids = self._inner.get_matching_rules(event)
        return deduplicate_with_order(
            [self._rule_id_to_rule[rid] for rid in rule_ids if rid in self._rule_id_to_rule]
        )

    def get_rule_id(self, rule: "Rule") -> int | None:
        return self._rule_to_id.get(id(rule))

    @property
    def rules(self) -> list["Rule"]:
        return list(self._rule_id_to_rule.values())

    @property
    def root(self):
        return None

    def print(self, *_):
        pass

    def get_size(self) -> int:
        return self._inner.size()
