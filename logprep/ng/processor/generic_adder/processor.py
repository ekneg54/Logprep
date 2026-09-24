"""
|PROCESSOR_NAME|
================
The `generic_adder` is a processor that adds new fields and values to documents based on a list.
The list resides inside a rule and/or inside a file.


Processor Configuration
^^^^^^^^^^^^^^^^^^^^^^^
..  code-block:: yaml
    :linenos:

    - genericaddername:
        type: generic_adder
        rules:
            - tests/testdata/rules/rules

.. autoclass:: logprep.processor.generic_adder.processor.GenericAdder.Config
   :members:
   :undoc-members:
   :inherited-members:
   :noindex:

.. automodule:: logprep.processor.generic_adder.rule
"""

# pylint: disable=import-error,no-name-in-module
# `logprep._rust.processor` is registered at runtime by the Rust extension.

import typing
from collections.abc import Sequence
from typing import ClassVar

from logprep._rust.processor import (  # pylint: disable=no-name-in-module
    PyGenericAdderSpecFactory,
)
from logprep.ng.abc.processor import Processor
from logprep.processor.generic_adder.rule import GenericAdderRule
from logprep.util.getter import RefreshableGetter


class GenericAdder(Processor):
    """Resolve values in documents by referencing a mapping list."""

    rule_class = GenericAdderRule
    spec_config_keys: ClassVar[frozenset[str]] = frozenset(
        {"add", "merge_with_target", "overwrite_target"}
    )

    def __init__(self, name: str, configuration: "Processor.Config") -> None:
        self._spec_factory = PyGenericAdderSpecFactory()
        super().__init__(name, configuration)

    @property
    def rules(self) -> Sequence[GenericAdderRule]:
        """Returns all rules"""
        return typing.cast(Sequence[GenericAdderRule], super().rules)

    async def setup(self):
        await super().setup()
        for rule in self.rules:
            rule.init_generic_adder(self._job_tag_for_cleanup)

    def _shut_down(self) -> None:
        RefreshableGetter.remove_callbacks_for_tag(self._job_tag_for_cleanup)
        return super()._shut_down()
