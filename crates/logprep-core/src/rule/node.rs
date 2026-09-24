use crate::filter::expression::FilterExpressionInner;

#[derive(Debug, Clone)]
pub struct NodeInner {
    pub expression: Option<FilterExpressionInner>,
    pub children: Vec<NodeInner>,
    pub matching_rule_ids: Vec<u64>,
}

impl NodeInner {
    pub fn new(expression: Option<FilterExpressionInner>) -> Self {
        Self {
            expression,
            children: Vec::new(),
            matching_rule_ids: Vec::new(),
        }
    }

    pub fn does_match(&self, document: &serde_json::Value) -> bool {
        match &self.expression {
            Some(expr) => expr.matches(document),
            None => true,
        }
    }

    pub fn add_child(&mut self, node: NodeInner) {
        self.children.push(node);
    }

    pub fn get_child_with_expression(&self, expr: &FilterExpressionInner) -> Option<&NodeInner> {
        self.children
            .iter()
            .find(|child| child.expression.as_ref().map_or(false, |e| e == expr))
    }

    pub fn get_child_with_expression_mut(
        &mut self,
        expr: &FilterExpressionInner,
    ) -> Option<&mut NodeInner> {
        self.children
            .iter_mut()
            .find(|child| child.expression.as_ref().map_or(false, |e| e == expr))
    }

    pub fn size(&self) -> usize {
        1 + self.children.iter().map(|c| c.size()).sum::<usize>()
    }
}
