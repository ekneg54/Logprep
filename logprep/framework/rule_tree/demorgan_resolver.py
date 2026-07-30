"""Module implements functionality to apply De Morgan's law on rule filter expressions"""

from logprep.filter.expression.filter_expression import (
    And,
    FilterExpression,
    Not,
    Or,
)


class DeMorganResolverException(Exception):
    """Raise if demorgan resolver encounters a problem."""


class DeMorganResolver:
    """Used to apply De Morgan's law on rule filter expressions"""

    def resolve(self, expression: FilterExpression) -> FilterExpression:
        """Parse NOT-expressions in given filter expression.

        This function resolves NOT-expressions found in the given filter expression according to
        De Morgan's law.

        Parameters
        ----------
        expression: FilterExpression
            Given filter expression to be parsed.

        Returns
        -------
        result: FilterExpression
            Resulting filter expression created by resolving NOT-expressions in the given filter
            expression.

        """
        if expression.expression_type == "Not":
            return self._resolve_not_expression(expression)
        if expression.expression_type in ("And", "Or"):
            return self._resolve_compound_expression(expression)

        return expression

    def _resolve_not_expression(self, not_expression: FilterExpression) -> FilterExpression:
        if not_expression.expression_type != "Not":
            raise DeMorganResolverException(
                f'Can\'t resolve expression "{not_expression}", since it\'s not of the type "NOT."'
            )

        if not_expression.children[0].expression_type not in ("And", "Or"):
            return not_expression

        compound_expression = not_expression.children[0]
        negated_children = tuple(Not(expression) for expression in compound_expression.children)

        if compound_expression.expression_type == "Or":
            expression = And(*negated_children)
        else:
            expression = Or(*negated_children)

        return self._resolve_compound_expression(expression)

    def _resolve_compound_expression(
        self, compound_expression: FilterExpression
    ) -> FilterExpression:
        resolved_children = tuple(
            self.resolve(expression) for expression in compound_expression.children
        )
        if compound_expression.expression_type == "And":
            return And(*resolved_children)
        return Or(*resolved_children)
