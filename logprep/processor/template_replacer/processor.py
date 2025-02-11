"""This module contains functionality replacing a text field using a template."""
from time import time
from typing import List
from logging import Logger, DEBUG

from os import walk
from os.path import isdir, realpath, join

from multiprocessing import current_process

from ruamel.yaml import YAML

from logprep.processor.base.processor import RuleBasedProcessor
from logprep.processor.template_replacer.rule import TemplateReplacerRule
from logprep.processor.base.exceptions import (
    NotARulesDirectoryError,
    InvalidRuleDefinitionError,
    InvalidRuleFileError,
)

from logprep.util.processor_stats import ProcessorStats
from logprep.util.time_measurement import TimeMeasurement

yaml = YAML(typ="safe", pure=True)


class TemplateReplacerError(BaseException):
    """Base class for TemplateReplacer related exceptions."""

    def __init__(self, name: str, message: str):
        super().__init__(f"TemplateReplacer ({name}): {message}")


class DuplicationError(TemplateReplacerError):
    """Raise if field already exists."""

    def __init__(self, name: str, skipped_fields: List[str]):
        message = (
            "The following fields already existed and "
            "were not overwritten by the TemplateReplacer: "
        )
        message += " ".join(skipped_fields)

        super().__init__(name, message)


class TemplateReplacer(RuleBasedProcessor):
    """Resolve values in documents by referencing a mapping list."""

    def __init__(
        self, name: str, tree_config: str, template_path: str, pattern: dict, logger: Logger
    ):
        super().__init__(name, tree_config, logger)
        self.ps = ProcessorStats()

        self._target_field = pattern["target_field"]
        self._target_field_split = self._target_field.split(".")
        self._fields = pattern["fields"]
        delimiter = pattern["delimiter"]
        allow_delimiter_field = pattern["allowed_delimiter_field"]
        allow_delimiter_index = self._fields.index(allow_delimiter_field)

        self._mapping = dict()
        with open(template_path, "r") as template_file:
            template = yaml.load(template_file)

        for key, value in template.items():
            split_key = key.split(delimiter)
            left, middle_and_right = (
                split_key[:allow_delimiter_index],
                split_key[allow_delimiter_index:],
            )
            middle = middle_and_right[: -(len(self._fields) - allow_delimiter_index - 1)]
            right = middle_and_right[-(len(self._fields) - allow_delimiter_index - 1) :]
            recombined_keys = left + ["-".join(middle)] + right

            if len(recombined_keys) != len(self._fields):
                raise TemplateReplacerError(
                    self._name,
                    f"Not enough delimiters in '{template_path}' " f"to populate {self._fields}",
                )

            try:
                _dict = self._mapping
                for idx, recombined_key in enumerate(recombined_keys):
                    if idx < len(self._fields) - 1:
                        if not _dict.get(recombined_key):
                            _dict[recombined_key] = dict()
                        _dict = _dict[recombined_key]
                    else:
                        _dict[recombined_key] = value

            except ValueError as error:
                raise TemplateReplacerError(
                    self._name, "template_replacer template is invalid!"
                ) from error

    # pylint: disable=arguments-differ
    def add_rules_from_directory(self, rule_paths: List[str]):
        """Add rules from given directory."""
        for path in rule_paths:
            if not isdir(realpath(path)):
                raise NotARulesDirectoryError(self._name, path)

            for root, _, files in walk(path):
                json_files = []
                for file in files:
                    if (file.endswith(".json") or file.endswith(".yml")) and not file.endswith(
                        "_test.json"
                    ):
                        json_files.append(file)
                for file in json_files:
                    rules = self._load_rules_from_file(join(root, file))
                    for rule in rules:
                        self._tree.add_rule(rule, self._logger)

        if self._logger.isEnabledFor(DEBUG):
            self._logger.debug(
                f"{self.describe()} loaded {self._tree.rule_counter} rules "
                f"({current_process().name})"
            )

        self.ps.setup_rules([None] * self._tree.rule_counter)
        # pylint: enable=arguments-differ

    def _load_rules_from_file(self, path: str):
        try:
            return TemplateReplacerRule.create_rules_from_file(path)
        except InvalidRuleDefinitionError as error:
            raise InvalidRuleFileError(self._name, path) from error

    def describe(self) -> str:
        return f"TemplateReplacer ({self._name})"

    @TimeMeasurement.measure_time("template_replacer")
    def process(self, event: dict):
        self._event = event

        for rule in self._tree.get_matching_rules(event):
            begin = time()
            self._apply_rules(event)
            processing_time = float("{:.10f}".format(time() - begin))
            idx = self._tree.get_rule_id(rule)
            self.ps.update_per_rule(idx, processing_time)

        self.ps.increment_processed_count()

    def _apply_rules(self, event):
        _dict = self._mapping
        for field in self._fields:
            dotted_field_value = self._get_dotted_field_value(event, field)
            if dotted_field_value is None:
                return

            value = str(dotted_field_value)
            _dict = _dict.get(value, None)
            if _dict is None:
                return

        if _dict is not None:
            if self._field_exists(event, self._target_field):
                _event = event
                for subfield in self._target_field_split[:-1]:
                    _event = _event[subfield]
                _event[self._target_field_split[-1]] = _dict

    @staticmethod
    def _field_exists(event: dict, dotted_field: str) -> bool:
        fields = dotted_field.split(".")
        dict_ = event
        for field in fields:
            if field in dict_:
                dict_ = dict_[field]
            else:
                return False
        return True
