# pylint: disable=missing-docstring
# pylint: disable=no-self-use
import re
import typing
from copy import deepcopy
from unittest import mock

import pytest

from logprep.processor.base.exceptions import FieldExistsWarning
from logprep.util.configuration import Configuration
from logprep.util.helper import (
    FieldValue,
    Missing,
    Skip,
    add_fields_to,
    camel_to_snake,
    field_value_validator,
    get_dotted_field_list,
    get_dotted_field_value,
    get_dotted_field_value_with_missing,
    get_dotted_field_values,
    get_field_value,
    get_field_value_no_slice,
    get_versions_string,
    has_dotted_field,
    join_dotted_fields,
    merge_collision_handler,
    merge_mutating_collision_handler,
    pop_dotted_field_value,
    reduce_field_value,
    snake_to_camel,
    transform_field_value,
)
from logprep.util.json_handling import is_json
from tests.testdata.metadata import path_to_config


class TestSentinels:
    def test_no_collission_with_missing(self):
        assert isinstance(Missing.MISSING, object)
        assert "MISSING" != Missing.MISSING
        assert "missing" != Missing.MISSING

    def test_no_collission_with_skip(self):
        assert isinstance(Skip.SKIP, object)
        assert "SKIP" != Skip.SKIP
        assert "skip" != Skip.SKIP


class TestCamelToSnake:
    @pytest.mark.parametrize(
        "camel_case, snake_case",
        [
            ("snakesOnAPlane", "snakes_on_a_plane"),
            ("SnakesOnAPlane", "snakes_on_a_plane"),
            ("snakes_on_a_plane", "snakes_on_a_plane"),
            ("IPhoneHysteria", "i_phone_hysteria"),
            ("iPhoneHysteria", "i_phone_hysteria"),
            ("GeoipEnricher", "geoip_enricher"),
        ],
    )
    def test_camel_to_snake(self, camel_case, snake_case):
        assert camel_to_snake(camel_case) == snake_case


class TestSnakeToCamel:
    @pytest.mark.parametrize(
        "camel_case, snake_case",
        [
            ("SnakesOnAPlane", "snakes_on_a_plane"),
            ("SnakesOnAPlane", "SnakesOnAPlane"),
            ("IPhoneHysteria", "i_phone_hysteria"),
            ("DatetimeExtractor", "datetime_extractor"),
            ("GenericAdder", "generic_adder"),
            ("Dissector", "dissector"),
            ("GeoipEnricher", "geoip_enricher"),
        ],
    )
    def test_snake_to_camel(self, camel_case, snake_case):
        assert snake_to_camel(snake_case) == camel_case


class TestIsJson:
    @pytest.mark.parametrize(
        "data, expected",
        [
            (
                """
                [
                    { "i": "am json"}
                ]
                """,
                True,
            ),
            (
                """
                key1: valid yaml but not json
                key2:
                    - key3: key3
                """,
                False,
            ),
            (
                """
                [
                    "i": "am not valid json but readable as yaml"
                ]
                """,
                False,
            ),
            (
                """
                filter: test_filter
                processor:
                    key1:
                    - key2: value2
                """,
                False,
            ),
        ],
    )
    def test_is_json_returns_expected(self, data, expected):
        with mock.patch("builtins.open", mock.mock_open(read_data=data)):
            assert is_json("mock_path") == expected


class TestGetDottedFieldValue:
    def test_get_dotted_field_value_nesting_depth_zero(self):
        event = {"dotted": "127.0.0.1"}
        dotted_field = "dotted"
        value = get_dotted_field_value(event, dotted_field)
        assert value == "127.0.0.1"

    def test_get_dotted_field_value_nesting_depth_one(self):
        event = {"dotted": {"field": "127.0.0.1"}}
        dotted_field = "dotted.field"
        value = get_dotted_field_value(event, dotted_field)
        assert value == "127.0.0.1"

    def test_get_dotted_field_value_nesting_depth_two(self):
        event = {"some": {"dotted": {"field": "127.0.0.1"}}}
        dotted_field = "some.dotted.field"
        value = get_dotted_field_value(event, dotted_field)
        assert value == "127.0.0.1"

    def test_get_dotted_field_value_with_escaping(self):
        event = {"dotted.field": "127.0.0.1", "dotted": {"field": "not me"}}
        dotted_field = "dotted\\.field"
        value = get_dotted_field_value(event, dotted_field)
        assert value == "127.0.0.1"

    def test_get_dotted_field_value_with_double_escaping(self):
        event = {"dotted\\.field": "127.0.0.1", "dotted": {"field": "not me"}}
        dotted_field = "dotted\\\\\\.field"
        value = get_dotted_field_value(event, dotted_field)
        assert value == "127.0.0.1", get_dotted_field_list(dotted_field)

    def test_get_dotted_field_value_nesting_depth_one_with_escaping(self):
        event = {"dotted": {"field.sub": "127.0.0.1"}}
        dotted_field = "dotted.field\\.sub"
        value = get_dotted_field_value(event, dotted_field)
        assert value == "127.0.0.1"

    def test_get_dotted_field_retrieves_sub_dict(self):
        event = {"some": {"dotted": {"field": "127.0.0.1"}}}
        dotted_field = "some.dotted"
        value = get_dotted_field_value(event, dotted_field)
        assert value == {"field": "127.0.0.1"}

    def test_get_dotted_field_retrieves_list(self):
        event = {"some": {"dotted": ["list", "with", "values"]}}
        dotted_field = "some.dotted"
        value = get_dotted_field_value(event, dotted_field)
        assert value == ["list", "with", "values"]

    def test_get_dotted_field_value_that_does_not_exist(self):
        event = {}
        dotted_field = "field"
        value = get_dotted_field_value(event, dotted_field)
        assert value is None

    def test_get_dotted_field_value_that_does_not_exist_from_nested_dict(self):
        event = {"some": {}}
        dotted_field = "some.dotted.field"
        value = get_dotted_field_value(event, dotted_field)
        assert value is None

    def test_get_dotted_field_value_that_matches_part_of_dotted_field(self):
        event = {"some": "do_not_match"}
        dotted_field = "some.dotted"
        value = get_dotted_field_value(event, dotted_field)
        assert value is None

    def test_get_dotted_field_value_key_matches_value(self):
        event = {"get": "dotted"}
        dotted_field = "get.dotted"
        value = get_dotted_field_value(event, dotted_field)
        assert value is None

    def test_get_dotted_field_with_list(self):
        event = {"get": ["dotted"]}
        dotted_field = "get.0"
        value = get_dotted_field_value(event, dotted_field)
        assert value == "dotted"

    def test_get_dotted_field_with_nested_list(self):
        event = {"get": ["dotted", ["does_not_matter", "target"]]}
        dotted_field = "get.1.1"
        value = get_dotted_field_value(event, dotted_field)
        assert value == "target"

    def test_get_dotted_field_with_list_not_found(self):
        event = {"get": ["dotted"]}
        dotted_field = "get.0.1"
        value = get_dotted_field_value(event, dotted_field)
        assert value is None

    def test_get_dotted_field_with_list_last_element(self):
        event = {"get": ["dotted", "does_not_matter", "target"]}
        dotted_field = "get.-1"
        value = get_dotted_field_value(event, dotted_field)
        assert value == "target"

    def test_get_dotted_field_with_out_of_bounds_index(self):
        event = {"get": ["dotted", "does_not_matter", "target"]}
        dotted_field = "get.3"
        value = get_dotted_field_value(event, dotted_field)
        assert value is None

    def test_get_dotted_fields_with_list_slicing(self):
        event = {"get": ["dotted", "does_not_matter", "target"]}
        dotted_field = "get.0:2"
        value = get_dotted_field_value(event, dotted_field)
        assert value == ["dotted", "does_not_matter"]

    def test_get_dotted_fields_with_list_slicing_short(self):
        event = {"get": ["dotted", "does_not_matter", "target"]}
        dotted_field = "get.:2"
        value = get_dotted_field_value(event, dotted_field)
        assert value == ["dotted", "does_not_matter"]

    def test_get_dotted_fields_reverse_order_with_slicing(self):
        event = {"get": ["dotted", "does_not_matter", "target"]}
        dotted_field = "get.::-1"
        value = get_dotted_field_value(event, dotted_field)
        assert value == ["target", "does_not_matter", "dotted"]

    def test_get_dotted_fiels_with_list_slicing_2(self):
        event = {"get": ["dotted", "does_not_matter", "target"]}
        dotted_field = "get.::2"
        value = get_dotted_field_value(event, dotted_field)
        assert value == ["dotted", "target"]


class TestPopDottedFieldValue:

    def test_removes_source_field_in_nested_structure_but_leaves_sibling(self):
        event = {"get": {"nested": "field", "other": "field"}}
        dotted_field = "get.nested"
        value = pop_dotted_field_value(event, dotted_field)
        assert value == "field"
        assert event == {"get": {"other": "field"}}

    def test_removes_plain_source_field(self):
        event = {"key": "field"}
        dotted_field = "key"
        value = pop_dotted_field_value(event, dotted_field)
        assert value == "field"
        assert not event

    def test_removes_plain_source_field_keep_empty(self):
        event = {"key": "field"}
        dotted_field = "key"
        value = pop_dotted_field_value(event, dotted_field, False)
        assert value == "field"
        assert not event

    def test_removes_source_field(self):
        event = {"get": {"nested": "field"}}
        dotted_field = "get.nested"
        value = pop_dotted_field_value(event, dotted_field)
        assert value == "field"
        assert not event

    def test_removes_source_field_keep_empty(self):
        event = {"get": {"nested": "field"}}
        dotted_field = "get.nested"
        value = pop_dotted_field_value(event, dotted_field, False)
        assert value == "field"
        assert event == {"get": {}}

    def test_removes_source_field2(self):
        event = {"get": {"very": {"deeply": {"nested": {"field": "value"}}}}}
        dotted_field = "get.very.deeply.nested"
        value = pop_dotted_field_value(event, dotted_field)
        assert value == {"field": "value"}
        assert not event

    def test_removes_source_field2_keep_empty(self):
        event = {"get": {"very": {"deeply": {"nested": {"field": "value"}}}}}
        dotted_field = "get.very.deeply.nested"
        value = pop_dotted_field_value(event, dotted_field, False)
        assert value == {"field": "value"}
        assert event == {"get": {"very": {"deeply": {}}}}

    def test_removes_source_field_with_escaping_in_node_key(self):
        event = {"get": {"comp\\lex.nested": {"key": "field"}}}
        dotted_field = "get.comp\\\\lex\\.nested.key"
        value = pop_dotted_field_value(event, dotted_field)
        assert value == "field"
        assert not event

    def test_removes_source_field_with_escaping_in_node_key_keep_empty(self):
        event = {"get": {"comp\\lex.nested": {"key": "field"}}}
        dotted_field = "get.comp\\\\lex\\.nested.key"
        value = pop_dotted_field_value(event, dotted_field, False)
        assert value == "field"
        assert event == {"get": {"comp\\lex.nested": {}}}

    def test_removes_source_field_with_escaping_in_leaf_key(self):
        event = {"get": {"nested": {"comp\\lex.key": "field"}}}
        dotted_field = "get.nested.comp\\\\lex\\.key"
        value = pop_dotted_field_value(event, dotted_field)
        assert value == "field"
        assert not event

    def test_removes_source_field_with_escaping_in_leaf_key_keep_empty(self):
        event = {"get": {"nested": {"comp\\lex.key": "field"}}}
        dotted_field = "get.nested.comp\\\\lex\\.key"
        value = pop_dotted_field_value(event, dotted_field, False)
        assert value == "field"
        assert event == {"get": {"nested": {}}}


class TestGetVersionString:
    def test_get_version_string(self):
        config = Configuration()
        config.version = "0.1.0"
        expected_pattern = (
            r"python version:\s+3\.\d+\.\d+\n"
            r"logprep version:\s+[^\s]+\n"
            r"configuration version:\s+0\.1\.0, None"
        )

        result = get_versions_string(config)
        assert re.search(expected_pattern, result)

    def test_get_version_string_with_config_source(self):
        config = Configuration.from_sources([path_to_config])
        expected_pattern = (
            r"python version:\s+3\.\d+\.\d+\n"
            r"logprep version:\s+[^\s]+\n"
            r"configuration version:\s+1,\s+file://[^\s]+/config\.yml"
        )

        result = get_versions_string(config)
        assert re.search(expected_pattern, result)

    def test_get_version_string_with_multiple_config_sources(self):
        config = Configuration.from_sources([path_to_config, path_to_config])
        expected_pattern = (
            r"python version:\s+3\.\d+\.\d+\n"
            r"logprep version:\s+[^\s]+\n"
            r"configuration version:\s+1,\s1,\s+file://[^\s]+/config\.yml,\s+file://[^\s]+/config\.yml"
        )

        result = get_versions_string(config)
        assert re.search(expected_pattern, result)

    def test_get_version_string_without_config(self):
        expected_pattern = (
            r"python version:\s+3\.\d+\.\d+\n"
            r"logprep version:\s+[^\s]+\n"
            r"configuration version:\s+no configuration found in file:///etc/logprep/pipeline.yml"
        )

        result = get_versions_string(None)
        assert re.search(expected_pattern, result)


class TestTransformFieldValue:

    def test_transform_complex_value(self):
        value = [
            "top-level",
            {
                "str": "whatever",
                "int": 42,
                "float": 13.37,
                "bool_t": True,
                "bool_f": False,
                "none": None,
                "soon_list": "make_list",
                "soon_dict": "make_dict",
                "list": [1, "1", 1.1, True, False, "make_list", "make_dict", None],
                "dict": {"sub": "anything"},
            },
        ]
        expected = [
            "value:top-level",
            {
                "key:str": "value:whatever",
                "key:int": 84,
                "key:float": 26.74,
                "key:bool_t": False,
                "key:bool_f": True,
                "key:none": None,
                "key:soon_list": [],
                "key:soon_dict": {},
                "key:list": [2, "value:1", 2.2, False, True, [], {}, None],
                "key:dict": {"key:sub": "value:anything"},
            },
        ]

        def transform_value(v: FieldValue) -> FieldValue:
            match (v):
                case "make_list":
                    return []
                case "make_dict":
                    return {}
                case str():
                    return f"value:{v}"
                case bool():
                    return not v
                case int() | float():
                    return 2 * v
                case None:
                    return None
                case _:
                    raise AssertionError("unexpected value encountered")

        result = transform_field_value(
            value, transform_key=lambda s: f"key:{s}", transform_value=transform_value
        )

        assert result == expected

    def test_error_on_illegal_field_value(self):
        value = typing.cast(FieldValue, tuple([1, 2, 3]))
        with pytest.raises(ValueError, match="unexpected type"):
            transform_field_value(
                value,
                transform_key=lambda x: x,
                transform_value=lambda x: x,
            )


class TestMergeCollisionHandlers:
    @pytest.mark.parametrize(
        "existing, incoming",
        [
            (
                {"existing": "value", "shared": "existing"},
                {"incoming": "value", "shared": "incoming"},
            ),
            (["existing"], ["incoming"]),
            (["existing"], "incoming"),
            ("existing", ["incoming"]),
        ],
    )
    def test_return_equal_values(self, existing: FieldValue, incoming: FieldValue):
        non_mutating_result = merge_collision_handler(
            "field", deepcopy(existing), deepcopy(incoming)
        )
        mutating_result = merge_mutating_collision_handler(
            "field", deepcopy(existing), deepcopy(incoming)
        )

        assert mutating_result == non_mutating_result


class TestReduceFieldValue:

    @staticmethod
    def collect_node_type_or_leaf_value(
        v: FieldValue, values: list[FieldValue | type[list] | type[dict]]
    ) -> list[FieldValue | type[list] | type[dict]]:
        if isinstance(v, (list, dict)):
            values.append(type(v))
        else:
            values.append(v)
        return values

    def test_reduce_complex_value(self):
        value = [
            {
                "str": "value",
                "int": 42,
                "float": 13.37,
                "bool_t": True,
                "bool_f": False,
                "none": None,
                "list": [1, "1", 1.1, True, False, None],
                "dict": {"key": "value"},
            }
        ]
        expected = [
            list,
            dict,
            "str",
            "value",
            "int",
            42,
            "float",
            13.37,
            "bool_t",
            True,
            "bool_f",
            False,
            "none",
            None,
            "list",
            list,
            1,
            "1",
            1.1,
            True,
            False,
            None,
            "dict",
            dict,
            "key",
            "value",
        ]

        result = reduce_field_value(self.collect_node_type_or_leaf_value, value, [])

        assert result == expected

    def test_error_on_illegal_field_value(self):
        value = typing.cast(FieldValue, tuple([1, 2, 3]))
        with pytest.raises(ValueError, match="unexpected type"):
            reduce_field_value(self.collect_node_type_or_leaf_value, value, [])


class TestFieldValueValidator:
    @pytest.mark.parametrize(
        "value, expected_error",
        [
            ({"nested": ["value", 42, 13.37, True, None, {"child": False}]}, None),
            ({1: "value"}, "add must have string keys"),
            ({"nested": [object()]}, "add must be a FieldValue, got object"),
        ],
    )
    def test_validates_recursive_field_values(self, value, expected_error):
        attribute = mock.Mock()
        attribute.name = "add"

        if expected_error:
            with pytest.raises(TypeError, match=expected_error):
                field_value_validator(None, attribute, value)
        else:
            field_value_validator(None, attribute, value)


class TestGetDottedFieldValueWithMissing:

    def test_returns_value_when_exists(self):
        event = {"a": {"b": "hello"}}
        result = get_dotted_field_value_with_missing(event, "a.b")
        assert result == "hello"

    def test_returns_missing_when_not_exists(self):
        event = {"a": 1}
        result = get_dotted_field_value_with_missing(event, "a.b")
        assert result is Missing.MISSING

    def test_returns_missing_for_empty_event(self):
        event = {}
        result = get_dotted_field_value_with_missing(event, "field")
        assert result is Missing.MISSING

    def test_returns_none_value_when_exists(self):
        event = {"a": None}
        result = get_dotted_field_value_with_missing(event, "a")
        assert result is None


class TestGetFieldValue:

    def test_simple_field(self):
        event = {"a": {"b": "world"}}
        result = get_field_value(event, ["a", "b"])
        assert result == "world"

    def test_missing_field(self):
        event = {"a": 1}
        result = get_field_value(event, ["a", "b"])
        assert result is Missing.MISSING

    def test_empty_fields(self):
        event = {"a": 1}
        result = get_field_value(event, [])
        assert result == event

    def test_deeply_nested(self):
        event = {"x": {"y": {"z": 42}}}
        result = get_field_value(event, ["x", "y", "z"])
        assert result == 42

    def test_none_value(self):
        event = {"a": None}
        result = get_field_value(event, ["a"])
        assert result is None


class TestGetFieldValueNoSlice:

    def test_simple_field(self):
        event = {"x": {"y": 42}}
        result = get_field_value_no_slice(event, ["x", "y"])
        assert result == 42

    def test_missing_field(self):
        event = {"x": 1}
        result = get_field_value_no_slice(event, ["x", "y"])
        assert result is Missing.MISSING

    def test_non_dict_returns_missing(self):
        event = {"x": "not_a_dict"}
        result = get_field_value_no_slice(event, ["x", "y"])
        assert result is Missing.MISSING

    def test_empty_fields(self):
        event = {"x": 1}
        result = get_field_value_no_slice(event, [])
        assert result == event


class TestGetDottedFieldValues:

    def test_batch_extraction(self):
        event = {"a": 1, "b": 2, "c": 3}
        result = get_dotted_field_values(event, ["a", "b", "c"])
        assert result == {"a": 1, "b": 2, "c": 3}

    def test_batch_with_missing_defaults_to_none(self):
        event = {"a": 1}
        result = get_dotted_field_values(event, ["a", "missing"])
        assert result == {"a": 1, "missing": None}

    def test_batch_with_on_missing_skip(self):
        event = {"a": 1}
        result = get_dotted_field_values(
            event, ["a", "missing"], on_missing=lambda _: Skip.SKIP
        )
        assert result == {"a": 1}
        assert "missing" not in result

    def test_batch_with_on_missing_default(self):
        event = {"a": 1}
        result = get_dotted_field_values(
            event, ["a", "missing"], on_missing=lambda _: "default"
        )
        assert result == {"a": 1, "missing": "default"}

    def test_batch_with_nested_fields(self):
        event = {"x": {"a": 10}, "y": {"b": 20}}
        result = get_dotted_field_values(event, ["x.a", "y.b"])
        assert result == {"x.a": 10, "y.b": 20}

    def test_empty_field_list(self):
        event = {"a": 1}
        result = get_dotted_field_values(event, [])
        assert result == {}


class TestHasDottedField:

    def test_existing_field(self):
        event = {"a": {"b": "val"}}
        assert has_dotted_field(event, "a.b") is True

    def test_missing_field(self):
        event = {"a": 1}
        assert has_dotted_field(event, "a.b") is False

    def test_none_value_allow_none(self):
        event = {"a": None}
        assert has_dotted_field(event, "a", allow_none=True) is True

    def test_none_value_not_allow_none(self):
        event = {"a": None}
        assert has_dotted_field(event, "a", allow_none=False) is False

    def test_empty_event(self):
        event = {}
        assert has_dotted_field(event, "a") is False

    def test_top_level_field(self):
        event = {"key": "value"}
        assert has_dotted_field(event, "key") is True


class TestJoinDottedFields:

    def test_simple(self):
        assert join_dotted_fields(["x.y", "z"]) == "x.y.z"

    def test_single(self):
        assert join_dotted_fields(["a"]) == "a"

    def test_empty(self):
        assert join_dotted_fields([]) == ""


class TestAddFieldsTo:

    def test_add_new_field(self):
        event = {}
        add_fields_to(event, {"hello": "world"})
        assert event == {"hello": "world"}

    def test_add_nested_field(self):
        event = {}
        add_fields_to(event, {"a.b.c": "deep"})
        assert event == {"a": {"b": {"c": "deep"}}}

    def test_overwrite_existing(self):
        event = {"key": "old"}
        add_fields_to(event, {"key": "new"}, overwrite_target=True)
        assert event == {"key": "new"}

    def test_skip_none(self):
        event = {}
        add_fields_to(event, {"a": None, "b": "keep"})
        assert event == {"b": "keep"}

    def test_no_skip_none(self):
        event = {}
        add_fields_to(event, {"a": None, "b": "keep"}, skip_none=False)
        assert event == {"a": None, "b": "keep"}

    def test_merge_dict(self):
        event = {"a": {"x": 1}}
        add_fields_to(event, {"a": {"y": 2}}, merge_with_target=True)
        assert event == {"a": {"x": 1, "y": 2}}

    def test_merge_list(self):
        event = {"a": [1, 2]}
        add_fields_to(event, {"a": [3, 4]}, merge_with_target=True)
        assert event == {"a": [1, 2, 3, 4]}

    def test_merge_scalar_to_list(self):
        event = {"a": [1]}
        add_fields_to(event, {"a": 2}, merge_with_target=True)
        assert event == {"a": [1, 2]}

    def test_merge_scalar_to_scalar_raises(self):
        event = {"a": 1}
        with pytest.raises(FieldExistsWarning):
            add_fields_to(event, {"a": 2}, merge_with_target=True)

    def test_field_exists_warning_on_conflict(self):
        from logprep.processor.base.exceptions import FieldExistsWarning

        event = {"key": "existing"}
        with pytest.raises(FieldExistsWarning):
            add_fields_to(event, {"key": "new"})

    def test_merge_and_overwrite_raises(self):
        event = {"a": 1}
        with pytest.raises(ValueError):
            add_fields_to(event, {"a": 2}, merge_with_target=True, overwrite_target=True)


class TestPopDottedFieldMissing:

    def test_pop_missing_field_returns_missing(self):
        event = {"a": 1}
        result = pop_dotted_field_value(event, "missing")
        assert result is Missing.MISSING

    def test_pop_missing_field_keep_empty(self):
        event = {"a": 1}
        result = pop_dotted_field_value(event, "missing", False)
        assert result is Missing.MISSING
        assert event == {"a": 1}
