import logging
from itertools import chain, zip_longest
from typing import Sequence

import luqum
from luqum.tree import (
    AndOperation,
    FieldGroup,
    Group,
    Not,
    OrOperation,
    Phrase,
    Prohibit,
    Range,
    Regex,
    SearchField,
    Word,
)

from logprep.abc.exceptions import LogprepException
from logprep.filter.expression.filter_expression import (
    Always,
    And,
    Exists,
    FilterExpression,
    FloatRangeFilterExpression,
    IntegerRangeFilterExpression,
)
from logprep.filter.expression.filter_expression import Not as NotExpression
from logprep.filter.expression.filter_expression import (
    Null,
    Or,
    RangeBoundary,
    RegExFilterExpression,
    SigmaFilterExpression,
    StringFilterExpression,
    StringRangeFilterExpression,
)
from logprep.util.helper import field_list_to_dotted_field, get_dotted_field_list

logger = logging.getLogger("LuceneFilter")


class LuceneFilterError(LogprepException):
    ...


class LuceneFilter:
    last_quotation_pattern = __import__("re").compile(r'((?:\\)+")$')
    quote_escaping_pattern = __import__("re").compile(r'(?:\\)+"')
    end_escaping_pattern = __import__("re").compile(r'((?:\\)+"[\s\)]+(?:AND|OR|NOT|$))')

    @staticmethod
    def create(query_string: str, special_fields: dict | None = None) -> FilterExpression:
        from logprep._rust import parse_lucene_query

        try:
            return parse_lucene_query(query_string, special_fields)
        except Exception as error:
            raise LuceneFilterError(f"{error} Expression: '{query_string}'") from error

    @staticmethod
    def _add_lucene_escaping(string: str) -> str:
        string = LuceneFilter._make_uneven_double_quotes_escaping(string)
        string = LuceneFilter._escape_ends_of_expressions(string)
        return string

    @staticmethod
    def _make_uneven_double_quotes_escaping(query_string: str) -> str:
        matches = LuceneFilter.quote_escaping_pattern.findall(query_string)
        for idx, match in enumerate(matches):
            cnt_backslashes = len(match) - 1
            if cnt_backslashes > 0:
                matches[idx] = f'{2 * matches[idx][:-1]}\\"'
        split = LuceneFilter.quote_escaping_pattern.split(query_string)
        query_string = "".join([x for x in chain.from_iterable(zip_longest(split, matches)) if x])
        return query_string

    @staticmethod
    def _escape_ends_of_expressions(query_string):
        split_string = LuceneFilter.end_escaping_pattern.split(query_string)
        new_string = ""
        for part in [split for split in split_string if split]:
            escaped_quotation = LuceneFilter.end_escaping_pattern.search(part)
            if escaped_quotation and part.startswith("\\"):
                for idx, char in enumerate(part):
                    if char == "\\":
                        new_string += char * 2
                    else:
                        new_string += part[idx:]
                        break
            else:
                new_string += part
        last_quotation = LuceneFilter.last_quotation_pattern.search(new_string)
        if last_quotation:
            cnt_backslashes = len(last_quotation.group()) - 1
            new_end = "\\" * cnt_backslashes * 2 + '"'
            new_string = new_string[: -(cnt_backslashes + 1)] + new_end
        return new_string


class LuceneTransformer:
    _special_fields_map: dict[str, type[RegExFilterExpression] | type[SigmaFilterExpression]] = {
        "regex_fields": RegExFilterExpression,
        "sigma_fields": SigmaFilterExpression,
    }

    find_unescaping_quote_pattern = __import__("re").compile(r'(?:\\)*"')
    find_unescaping_end_pattern = __import__("re").compile(r"(?:\\)*\Z")

    def __init__(self, tree: luqum.tree, special_fields: dict | None = None):
        self._tree = tree
        self._special_fields = {}
        special_fields = special_fields if special_fields else {}
        for key in self._special_fields_map:
            self._special_fields[key] = special_fields.get(key, [])
        self._last_search_field = None

    def build_filter(self) -> FilterExpression:
        return self._parse_tree(self._tree)

    def _parse_tree(self, tree: luqum.tree) -> FilterExpression:
        if isinstance(tree, OrOperation):
            return Or(*self._collect_children(tree))
        if isinstance(tree, AndOperation):
            return And(*self._collect_children(tree))
        if isinstance(tree, Not):
            return NotExpression(*self._collect_children(tree))
        if isinstance(tree, Group):
            return self._parse_tree(tree.children[0])
        if isinstance(tree, Range):
            if self._last_search_field is None:
                raise LuceneFilterError(f'The expression "{str(tree)}" is invalid!')
            key = get_dotted_field_list(self._last_search_field)
            return self._parse_range(key, tree)
        if isinstance(tree, SearchField):
            if isinstance(tree.expr, FieldGroup):
                self._last_search_field = tree.name
                parsed = self._parse_tree(tree.expr.children[0])
                self._last_search_field = None
                return parsed
            return self._create_field(tree)
        if isinstance(tree, (Word, Phrase, Regex)):
            if self._last_search_field is not None:
                return self._create_field_group_expression(tree, self._last_search_field)
            return self._create_value_expression(tree)
        raise LuceneFilterError(f'The expression "{str(tree)}" is invalid!')

    def _parse_range(self, key: Sequence[str], expr: Range) -> FilterExpression:
        lower_value = self._get_range_boundary_value(expr.low)
        upper_value = self._get_range_boundary_value(expr.high)
        for range_parser in (
            self._parse_integer_range,
            self._parse_float_range,
            self._parse_string_range,
        ):
            range_filter_expression = range_parser(key, lower_value, upper_value, expr.include_low, expr.include_high, expr)
            if range_filter_expression is not None:
                return range_filter_expression
        raise LuceneFilterError(f'The expression "{expr}" is invalid!')

    @staticmethod
    def _parse_integer_range(key, lower_value, upper_value, include_lower_bound, include_upper_bound, expr):
        try:
            lower_bound = int(lower_value)
            upper_bound = int(upper_value)
        except ValueError:
            return None
        LuceneTransformer._validate_range_boundaries(lower_bound, upper_bound, expr)
        return IntegerRangeFilterExpression(key, lower_bound, upper_bound, include_lower_bound, include_upper_bound)

    @staticmethod
    def _parse_float_range(key, lower_value, upper_value, include_lower_bound, include_upper_bound, expr):
        import math
        try:
            lower_bound = float(lower_value)
            upper_bound = float(upper_value)
        except ValueError:
            return None
        if not math.isfinite(lower_bound) or not math.isfinite(upper_bound):
            raise LuceneFilterError(f'The expression "{expr}" is invalid. Range boundaries must be finite numbers.')
        LuceneTransformer._validate_range_boundaries(lower_bound, upper_bound, expr)
        return FloatRangeFilterExpression(key, lower_bound, upper_bound, include_lower_bound, include_upper_bound)

    @staticmethod
    def _parse_string_range(key, lower_value, upper_value, include_lower_bound, include_upper_bound, expr):
        if lower_value == "*" or upper_value == "*":
            raise LuceneFilterError(f'The expression "{expr}" is invalid!')
        if LuceneTransformer._is_finite_numeric_range_boundary(lower_value) or LuceneTransformer._is_finite_numeric_range_boundary(upper_value):
            return None
        LuceneTransformer._validate_range_boundaries(lower_value, upper_value, expr)
        return StringRangeFilterExpression(key, lower_value, upper_value, include_lower_bound, include_upper_bound)

    @staticmethod
    def _is_finite_numeric_range_boundary(value: str) -> bool:
        import math
        try:
            numeric_value = float(value)
        except ValueError:
            return False
        return math.isfinite(numeric_value)

    @staticmethod
    def _get_range_boundary_value(token: luqum.tree) -> str:
        if isinstance(token, Word):
            return token.value
        if isinstance(token, Phrase):
            return token.value.strip('"')
        if isinstance(token, Prohibit) and len(token.children) == 1:
            child = token.children[0]
            if isinstance(child, Word):
                return f"-{child.value}"
        raise LuceneFilterError(f'The range boundary "{token}" is invalid!')

    @staticmethod
    def _validate_range_boundaries(lower_bound, upper_bound, expr):
        if lower_bound > upper_bound:
            raise LuceneFilterError("The lower range boundary must not exceed " f'the upper range boundary: "{expr}"')

    def _create_field_group_expression(self, tree, dotted_field):
        key = get_dotted_field_list(dotted_field)
        value = self._strip_quote_from_string(tree.value)
        value = self._remove_lucene_escaping(value)
        if isinstance(tree, Regex):
            return self._get_filter_expression_regex(key, value)
        return self._get_filter_expression(key, value)

    def _collect_children(self, tree):
        expressions = []
        for child in tree.children:
            expressions.append(self._parse_tree(child))
        return expressions

    def _create_field(self, tree):
        key = get_dotted_field_list(tree.name)
        if isinstance(tree.expr, (Phrase, Word)):
            if tree.expr.value == "null":
                return Null(key)
            value = self._strip_quote_from_string(tree.expr.value)
            value = self._remove_lucene_escaping(value)
            return self._get_filter_expression(key, value)
        if isinstance(tree.expr, Regex):
            if tree.expr.value == "null":
                return Null(key)
            value = self._strip_quote_from_string(tree.expr.value)
            value = self._remove_lucene_escaping(value)
            return self._get_filter_expression_regex(key, value)
        if isinstance(tree.expr, Range):
            return self._parse_range(key, tree.expr)
        raise LuceneFilterError(f'The expression "{str(tree)}" is invalid!')

    @staticmethod
    def _check_key_and_modifier(key, value):
        key_and_modifier = key[-1].split("|")
        if len(key_and_modifier) == 2:
            if key_and_modifier[-1] == "re":
                return RegExFilterExpression([*key[:-1], *key_and_modifier[:-1]], value)
        return None

    def _get_filter_expression(self, key, value):
        key_and_modifier_check = LuceneTransformer._check_key_and_modifier(key, value)
        if key_and_modifier_check is not None:
            return key_and_modifier_check
        dotted_field = field_list_to_dotted_field(key)
        if self._special_fields.items():
            for sf_key, sf_value in self._special_fields.items():
                if sf_value is True or dotted_field in sf_value:
                    if sf_key == "regex_fields":
                        logger.warning("[Deprecated]: regex_fields are no longer necessary. Use Lucene regex annotation.")
                    return self._special_fields_map[sf_key](key, value)
        return StringFilterExpression(key, value)

    def _get_filter_expression_regex(self, key, value):
        key_and_modifier_check = LuceneTransformer._check_key_and_modifier(key, value)
        if key_and_modifier_check is not None:
            return key_and_modifier_check
        value = value.strip("/")
        return RegExFilterExpression(key, value)

    @staticmethod
    def _create_value_expression(word):
        value = get_dotted_field_list(word.value)
        if value == ["*"]:
            return Always(True)
        return Exists(value)

    @staticmethod
    def _strip_quote_from_string(string: str) -> str:
        if (string[0] == string[-1]) and (string[0] in ["'", '"']):
            return string[1:-1]
        return string

    @staticmethod
    def _remove_lucene_escaping(string: str) -> str:
        string = LuceneTransformer._remove_escaping_from_end_of_expression(string)
        string = LuceneTransformer._remove_uneven_double_quotes_escaping(string)
        string = LuceneTransformer._remove_one_escaping_from_quotes(string)
        return string

    @staticmethod
    def _remove_escaping_from_end_of_expression(string: str) -> str:
        escaping_end = LuceneTransformer.find_unescaping_end_pattern.search(string)
        if escaping_end is None:
            return string
        backslashes_end_cnt = len(escaping_end.group())
        string = string[: escaping_end.start()] + "\\" * (backslashes_end_cnt // 2)
        escaping_end = LuceneTransformer.find_unescaping_end_pattern.search(string)
        if escaping_end:
            new_escaping = "\\" * ((len(escaping_end.group()) - 1) // 2)
            string = f"{string[:escaping_end.start()]}{new_escaping}"
        return string

    @staticmethod
    def _remove_uneven_double_quotes_escaping(string: str) -> str:
        matches = LuceneTransformer.find_unescaping_quote_pattern.findall(string)
        if matches is None:
            return string
        for idx, match in enumerate(matches):
            cnt_backslashes = len(match) - 1
            if cnt_backslashes >= 3:
                matches[idx] = f'{matches[idx][:(cnt_backslashes - 2) // 2]}\\"'
        split = LuceneTransformer.find_unescaping_quote_pattern.split(string)
        string = "".join([x for x in chain.from_iterable(zip_longest(split, matches)) if x])
        return string

    @staticmethod
    def _remove_one_escaping_from_quotes(string: str) -> str:
        matches = LuceneTransformer.find_unescaping_quote_pattern.findall(string)
        if matches is None:
            return string
        for idx, match in enumerate(matches):
            len_match = len(match) - 1
            if len_match >= 1:
                matches[idx] = matches[idx][1:]
        split = LuceneTransformer.find_unescaping_quote_pattern.split(string)
        string = "".join([x for x in chain.from_iterable(zip_longest(split, matches)) if x])
        return string
