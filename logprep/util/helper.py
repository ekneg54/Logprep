"""This module contains helper functions that are shared by different modules."""

import functools
import re
import sys
import typing
from enum import Enum, auto
from functools import partial
from importlib.metadata import version
from os import remove
from string import Template
from typing import (
    TYPE_CHECKING,
    Callable,
    Iterable,
    Optional,
    TypeAlias,
    TypeVar,
    Union,
)

from attrs import Attribute

from logprep._rust import (  # noqa: F401
    add_fields_to,
    field_list_to_dotted_field,
    get_dotted_field_list,
    get_dotted_field_value,
    get_dotted_field_value_with_missing,
    get_dotted_field_values,
    get_field_value,
    get_field_value_no_slice,
    has_dotted_field,
    join_dotted_fields,
    pop_dotted_field_value,
)
from logprep.processor.base.exceptions import FieldExistsWarning  # noqa: F401
from logprep.util.defaults import DEFAULT_CONFIG_LOCATION

if TYPE_CHECKING:  # pragma: no cover
    from logprep.ng.util.configuration import Configuration as NgConfiguration
    from logprep.processor.base.rule import Rule
    from logprep.util.configuration import Configuration


class Missing(Enum):
    """Sentinel type for indicating missing fields."""

    MISSING = auto()


MISSING = Missing.MISSING  # pylint: disable=invalid-name
"""Sentinel value for indicating missing fields."""


class Skip(Enum):
    """Sentinel type for method instrumentation to skip fields."""

    SKIP = auto()


SKIP = Skip.SKIP  # pylint: disable=invalid-name
"""Sentinel value for method instrumentation to skip fields."""


FieldValue: TypeAlias = Union[
    dict[str, "FieldValue"], list["FieldValue"], str, int, float, bool, None
]

FieldRef: TypeAlias = str

T = TypeVar("T")


class DottedTemplate(Template):
    braceidpattern = r"(?a:(?:\\.|[^.$\\{}])+(?:\.(?:\\.|[^.$\\{}])+)*)"


# =============================================================================
# Thin Wrappers (Step 1h)
# =============================================================================

append_as_list = partial(add_fields_to, merge_with_target=True)


def _add_field_to_silent_fail(
    event, field, rule=None, merge_with_target=False, overwrite_target=False
):
    """Wrapper for single field add with silent failure. Used by metrics instrumentation."""
    try:
        add_fields_to(
            event,
            dict([field]),
            rule=rule,
            merge_with_target=merge_with_target,
            overwrite_target=overwrite_target,
        )
    except FieldExistsWarning as error:
        return error.skipped_fields[0]
    return None


def add_and_overwrite(event, fields, rule, *_):
    """Wrapper for add_field_to with overwrite_target=True."""
    add_fields_to(event, fields, rule, overwrite_target=True)


def append(event, field, separator, rule):
    """Appends to event"""
    target_field, content = list(field.items())[0]
    target_value = get_dotted_field_value(event, target_field)
    if not isinstance(target_value, list):
        target_value = "" if target_value is None else target_value
        target_value = f"{target_value}{separator}{content}"
        add_and_overwrite(event, fields={target_field: target_value}, rule=rule)
    else:
        append_as_list(event, field)


def get_source_fields_dict(event, rule):
    """Returns a dict with dotted fields as keys and target values as values"""
    source_fields = rule.source_fields
    return {field: get_dotted_field_value(event, field) for field in source_fields}


def copy_fields_to_event(
    target_event,
    source_event,
    dotted_field_names,
    *,
    skip_missing=True,
    merge_with_target=False,
    overwrite_target=False,
    rule=None,
):
    """
    Copies fields from source_event to target_event.

    Parameters
    ----------
    target_event : dict
        The field dictionary where fields are being added to in-place
    source_event : dict
        The field dictionary where field values are being read from
    dotted_field_names : Iterable[str]
        The list of (potentially dotted) field names to copy
    skip_missing : bool, optional
        Controls whether missing fields should be skipped or defaulted to None, by default True
    merge_with_target : bool, optional
        Controls whether already existing fields should be merged as a list, by default False
    overwrite_target : bool, optional
        Controls whether already existing fields should be overwritten, by default False
    rule : Rule, optional
        Contextual info for error handling, by default None
    """
    on_missing_result = SKIP if skip_missing else None
    source_fields = get_dotted_field_values(
        source_event, dotted_field_names, on_missing=lambda _: on_missing_result
    )
    add_fields_to(
        target_event,
        source_fields,
        rule=rule,
        overwrite_target=overwrite_target,
        merge_with_target=merge_with_target,
        skip_none=False,
    )


# =============================================================================
# Non-dotted-field Helpers (unchanged)
# =============================================================================


def recursive_compare(test_output, expected_output):
    """Recursively compares given test_output against an expected output."""
    result = None

    if not isinstance(test_output, type(expected_output)):
        return test_output, expected_output

    if isinstance(test_output, dict) and isinstance(expected_output, dict):
        if sorted(test_output.keys()) != sorted(expected_output.keys()):
            return sorted(test_output.keys()), sorted(expected_output.keys())

        for key in test_output.keys():
            result = recursive_compare(test_output[key], expected_output[key])
            if result:
                return result

    elif isinstance(test_output, list) and isinstance(expected_output, list):
        for index, _ in enumerate(test_output):
            result = recursive_compare(test_output[index], expected_output[index])
            if result:
                return result

    else:
        if test_output != expected_output:
            result = test_output, expected_output

    return result


def remove_file_if_exists(test_output_path):
    """Remove existing file."""
    try:
        remove(test_output_path)
    except FileNotFoundError:
        pass


def camel_to_snake(camel: str) -> str:
    """Ensures that the input string is snake_case"""

    _underscorer1 = re.compile(r"(.)([A-Z][a-z]+)")
    _underscorer2 = re.compile("([a-z0-9])([A-Z])")

    subbed = _underscorer1.sub(r"\1_\2", camel)
    return _underscorer2.sub(r"\1_\2", subbed).lower()


def snake_to_camel(snake: str) -> str:
    """Ensures that the input string is CamelCase"""

    components = snake.split("_")
    if len(components) == 1:
        camel = components[0]
        return f"{camel[0].upper()}{camel[1:]}"

    camel = "".join(component.title() for component in components)
    return camel


def get_versions_string(
    config: Optional["Configuration"] | Optional["NgConfiguration"] = None,
) -> str:
    """
    Prints the version and exists. If a configuration was found then it's version
    is printed as well
    """
    padding = 25
    version_string = f"{'python version:'.ljust(padding)}{sys.version.split()[0]}"
    version_string += f"\n{'logprep version:'.ljust(padding)}{version('logprep')}"
    if config:
        config_version = (
            f"{config.version}, {', '.join(config.config_paths) if config.config_paths else 'None'}"
        )
    else:
        config_version = f"no configuration found in {', '.join([DEFAULT_CONFIG_LOCATION])}"
    version_string += f"\n{'configuration version:'.ljust(padding)}{config_version}"
    return version_string


def deduplicate_with_order(items: Iterable[T]) -> list[T]:
    """Deduplicates the given items while maintaining their order

    Parameters
    ----------
    items : Iterable[T]
        The items to be deduplicated

    Returns
    -------
    list[T]
        The deduplicated list
    """
    return list(dict.fromkeys(items))


def resolve_template(
    template: str,
    data: dict[str, FieldValue],
    serialize: Callable[[FieldValue], str] = str,
) -> str:
    """Resolve a string template by substituting placeholders in the form `${nested.key}`
    with their respective values taken from the data dict.
    This method follows a naive approach and attempts to substitute all keys from the data dict
    in the template.
    If there are any placeholders which can not be resolved this way, they are kept as-is and no
    error is raised.

    Parameters
    ----------
    template : str
        The template with dollar-curly-bracket based placeholders to be replaced
    data : dict[str, FieldValue]
        The data source for substituting placeholders
    serialize : Callable[[FieldValue], str], optional
        Used to convert :code:`FieldValue` to a string representation, by default str

    Returns
    -------
    str
        The resolved template string
    """
    result = template
    for key, value in data.items():
        escaped_key = key.replace("\\", "\\\\").replace(".", "\\.")
        pattern = r"\$\{(" + rf"{escaped_key}" + r")\}"
        result = re.sub(pattern, serialize(value), result)
    return result


def create_template_resolver(
    data: dict[str, FieldValue],
    serialize: Callable[[FieldValue], str] = str,
) -> Callable[[str], str]:
    """Prepares a template resolver for substituting placeholders in the form `${nested.key}`
    with their respective values taken from the data dict.
    This method follows a naive approach and attempts to substitute all keys from the data dict
    in the template.
    If there are any placeholders which can not be resolved this way, they are kept as-is and no
    error is raised.

    Parameters
    ----------
    data : dict[str, FieldValue]
        The data source for substituting placeholders
    serialize : Callable[[FieldValue], str], optional
        Used to convert :code:`FieldValue` to a string representation, by default str

    Returns
    -------
    Callable[[str], str]
        The template resolver, transforming a string by substituting all placeholders for which it has values
    """
    resolve_dict = {}
    for key, value in data.items():
        escaped_key = key.replace("\\", "\\\\").replace(".", "\\.")
        pattern = r"\$\{(" + rf"{escaped_key}" + r")\}"
        resolve_dict[pattern] = serialize(value)

    def resolve(template: str) -> str:
        result = template
        for pattern, value in resolve_dict.items():
            result = re.sub(pattern, value, result)
        return result

    return resolve


def reduce_field_value(func: Callable[[FieldValue, T], T], data: FieldValue, initial: T) -> T:
    """Traverses the given :code:`FieldValue` and calls the given function per element.
    :code:`dict` and :code:`list` are hereby considered nodes and their keys, values and items
    are visited in the process.

    Parameters
    ----------
    func : Callable[[FieldValue, T], T]
        Callback for handling the element and integrating it into the constructed result
    data : FieldValue
        The potentially nested :code:`FieldValue` data structure
    initial : T
        The initial result value being modified on each callback call

    Returns
    -------
    T
        The result value after being transformed through all callback invocations

    Raises
    ------
    ValueError
        If an unexpected type is encountered
    """
    result = initial
    match (data):
        case dict():
            result = func(data, result)
            for key, value in data.items():
                result = reduce_field_value(func, key, result)
                result = reduce_field_value(func, value, result)
        case list():
            result = func(data, result)
            for item in data:
                result = reduce_field_value(func, item, result)
        case str() | int() | float() | bool() | None:
            result = func(data, result)
        case _:
            raise ValueError(f"unexpected type encountered: {type(data)}")
    return result


class FieldCollisionError(ValueError):
    """Raised when transformed dictionary keys cannot be resolved"""

    def __init__(self, key: str, existing_type: type, incoming_type: type) -> None:
        self.key = key
        self.existing_type = existing_type
        self.incoming_type = incoming_type
        super().__init__(
            f"collision for transformed key {key!r}: "
            f"cannot merge {existing_type.__name__} with {incoming_type.__name__}"
        )


def transform_field_value(
    data: FieldValue,
    /,
    transform_value: Callable[[str | int | float | bool | None], FieldValue],
    transform_key: Callable[[str], str],
    collision_handler: Callable[[str, FieldValue, FieldValue], FieldValue] | None = None,
) -> FieldValue:
    """Transforms a field value by mapping all leafs (not :code:`dict` and :code:`list`)
    to new values.

    Parameters
    ----------
    transform_value : Callable[[FieldValue], FieldValue]
        Transforms items of lists, values of dicts and all plain value types to a new value
    transform_key : Callable[[str], str]
        Transforms keys of dicts to a new value
    data : FieldValue
        The potentially complex :code:`FieldValue` to traverse and transform

    Returns
    -------
    FieldValue
        The transformed :code:`FieldValue`

    Raises
    ------
    ValueError
        If an unexpected type is encountered
    """

    transform_recursive = functools.partial(
        transform_field_value,
        transform_value=transform_value,
        transform_key=transform_key,
        collision_handler=collision_handler,
    )

    match data:
        case dict():
            transformed: dict[str, FieldValue] = {}
            for key, value in data.items():
                transformed_key = transform_key(key)
                transformed_value = transform_recursive(value)

                if transformed_key in transformed and collision_handler is not None:
                    transformed[transformed_key] = collision_handler(
                        transformed_key, transformed[transformed_key], transformed_value
                    )
                else:
                    transformed[transformed_key] = transformed_value

            return transformed
        case list():
            return [transform_recursive(item) for item in data]
        case str() | int() | float() | bool() | None:
            return transform_value(data)
        case _:
            raise ValueError(f"unexpected type encountered: {type(data)}")


def keep_incoming_collision_handler(
    _key: str,
    _existing: FieldValue,
    incoming: FieldValue,
) -> FieldValue:
    """Resolve a key collision by retaining the incoming value."""
    return incoming


def merge_collision_handler(
    key: str,
    existing: FieldValue,
    incoming: FieldValue,
) -> FieldValue:
    """Resolve compatible key collisions by returning a merged value.

    The inputs are not mutated. Raises :class:`FieldCollisionError` for incompatible
    value types.
    """
    match existing, incoming:
        case dict(), dict():
            return {**existing, **incoming}
        case list(), list():
            return [*existing, *incoming]
        case list(), str() | int() | float() | bool() | None:
            return [*existing, incoming]
        case str() | int() | float() | bool() | None, list():
            return [existing, *incoming]
        case _:
            raise FieldCollisionError(key, type(existing), type(incoming))


def merge_mutating_collision_handler(
    key: str,
    existing: FieldValue,
    incoming: FieldValue,
) -> FieldValue:
    """Resolve compatible key collisions by mutating and returning a value.

    This avoids allocating a replacement container where possible. Raises
    :class:`FieldCollisionError` for incompatible value types.
    """
    match existing, incoming:
        case dict(), dict():
            existing.update(incoming)
            return existing
        case list(), list():
            existing.extend(incoming)
            return existing
        case list(), str() | int() | float() | bool() | None:
            existing.append(incoming)
            return existing
        case str() | int() | float() | bool() | None, list():
            incoming.insert(0, existing)
            return incoming
        case _:
            raise FieldCollisionError(key, type(existing), type(incoming))


def field_value_validator(_: object, attribute: Attribute, value: object) -> None:
    """Validate that an attrs attribute recursively contains only ``FieldValue`` values.

    Dictionaries must use strings as keys. Unsupported values raise :class:`TypeError`.
    """
    match value:
        case None | str() | bool() | int() | float():
            return
        case list():
            for item in value:
                field_value_validator(_, attribute, item)
        case dict():
            if not all(isinstance(key, str) for key in value):
                raise TypeError(f"{attribute.name} must have string keys")
            for item in value.values():
                field_value_validator(_, attribute, item)
        case _:
            raise TypeError(f"{attribute.name} must be a FieldValue, got {type(value).__name__}")
