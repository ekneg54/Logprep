from logprep._rust import PyFilterExpression as _PyFilterExpression

# Re-export FilterExpression
FilterExpression = _PyFilterExpression

# Factory functions
from logprep._rust import (
    filter_expression as Always,
    filter_expression_not as Not,
    filter_expression_and as And,
    filter_expression_or as Or,
    filter_expression_string as StringFilterExpression,
    filter_expression_wildcard as WildcardStringFilterExpression,
    filter_expression_sigma as SigmaFilterExpression,
    filter_expression_integer as IntegerFilterExpression,
    filter_expression_float as FloatFilterExpression,
    filter_expression_integer_range as IntegerRangeFilterExpression,
    filter_expression_float_range as FloatRangeFilterExpression,
    filter_expression_string_range as StringRangeFilterExpression,
    filter_expression_regex as RegExFilterExpression,
    filter_expression_exists as Exists,
    filter_expression_null as Null,
)

# Exception classes
class FilterExpressionError(Exception):
    ...


class KeyDoesNotExistError(FilterExpressionError):
    ...


# Backward compat for isinstance checks — use expression_type instead
# These names are kept for import compatibility (type annotations)
class KeyBasedFilterExpression:
    ...


class CompoundFilterExpression:
    def __init__(self, *children):
        self.children = list(children)
        self.expression_type = "CompoundFilterExpression"


RangeBoundary = type("RangeBoundary", (), {})

# _get_value: standalone helper for test compatibility
def _get_value(key, document):
    if not key:
        raise KeyDoesNotExistError()
    current = document
    for item in key:
        if not isinstance(current, dict):
            raise KeyDoesNotExistError()
        if item not in current:
            raise KeyDoesNotExistError()
        current = current[item]
    return current


FilterExpression._get_value = staticmethod(_get_value)
