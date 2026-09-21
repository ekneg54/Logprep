"""
|PROCESSOR_NAME|
================

The `key_checker` processor checks if all field names in a provided list are
given in the processed event.

Processor Configuration
^^^^^^^^^^^^^^^^^^^^^^^
..  code-block:: yaml
    :linenos:

    - keycheckername:
        type: key_checker
        rules:
            - tests/testdata/rules/rules

.. autoclass:: logprep.processor.key_checker.processor.KeyChecker.Config
   :members:
   :undoc-members:
   :inherited-members:
   :noindex:

.. automodule:: logprep.processor.key_checker.rule
"""

# pylint: disable=import-error,no-name-in-module
# `logprep._rust.processor` is registered at runtime by the Rust extension.

from typing import ClassVar

from logprep._rust.processor import PyKeyCheckerSpecFactory  # pylint: disable=no-name-in-module
from logprep.ng.abc.processor import Processor
from logprep.processor.key_checker.rule import KeyCheckerRule


class KeyChecker(Processor):
    """Checks if all keys of a given List are in the event"""

    rule_class = KeyCheckerRule
    spec_config_keys: ClassVar[frozenset[str]] = frozenset(
        {"source_fields", "target_field", "overwrite_target", "merge_with_target"}
    )

    def __init__(self, name: str, configuration: "Processor.Config") -> None:
        self._spec_factory = PyKeyCheckerSpecFactory()
        super().__init__(name, configuration)
