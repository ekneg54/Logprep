use serde_json::Value;

use crate::filter::expression::FilterExpressionInner;

use super::node::NodeInner;

#[derive(Debug, Clone)]
pub struct TreeInner {
    root: NodeInner,
    rule_count: usize,
}

impl TreeInner {
    pub fn new() -> Self {
        Self {
            root: NodeInner::new(None),
            rule_count: 0,
        }
    }

    pub fn add_rule(&mut self, segments: &[FilterExpressionInner], rule_id: u64) {
        Self::add_rule_to_node(&mut self.root, segments, rule_id);
        self.rule_count += 1;
    }

    fn add_rule_to_node(
        node: &mut NodeInner,
        segments: &[FilterExpressionInner],
        rule_id: u64,
    ) {
        if segments.is_empty() {
            if !node.matching_rule_ids.contains(&rule_id) {
                node.matching_rule_ids.push(rule_id);
            }
            return;
        }
        let expr = &segments[0];
        let found_idx = node.children.iter().position(|child| {
            child.expression.as_ref().map_or(false, |e| e == expr)
        });
        if let Some(idx) = found_idx {
            Self::add_rule_to_node(&mut node.children[idx], &segments[1..], rule_id);
        } else {
            node.children.push(NodeInner::new(Some(expr.clone())));
            let idx = node.children.len() - 1;
            Self::add_rule_to_node(&mut node.children[idx], &segments[1..], rule_id);
        }
    }

    pub fn get_matching_rules(&self, event: &Value) -> Vec<u64> {
        let mut matches = Vec::new();
        self.collect_matches(&self.root, event, &mut matches);
        let mut seen = std::collections::HashSet::new();
        matches.retain(|id| seen.insert(*id));
        matches
    }

    fn collect_matches(&self, node: &NodeInner, event: &Value, matches: &mut Vec<u64>) {
        for child in &node.children {
            if child.does_match(event) {
                matches.extend_from_slice(&child.matching_rule_ids);
                self.collect_matches(child, event, matches);
            }
        }
    }

    pub fn rule_count(&self) -> usize {
        self.rule_count
    }

    pub fn size(&self) -> usize {
        self.root.size()
    }

    pub fn root(&self) -> &NodeInner {
        &self.root
    }
}

impl Default for TreeInner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::expression::FilterExpressionInner;
    use serde_json::json;

    #[test]
    fn empty_tree_returns_no_rules() {
        let tree = TreeInner::new();
        let event = json!({"field": "value"});
        assert!(tree.get_matching_rules(&event).is_empty());
    }

    #[test]
    fn simple_rule_matches() {
        let mut tree = TreeInner::new();
        let expr = FilterExpressionInner::String {
            key: vec!["field".into()],
            expected: "value".into(),
        };
        tree.add_rule(&[expr], 1);
        let event = json!({"field": "value"});
        assert_eq!(tree.get_matching_rules(&event), vec![1]);
    }

    #[test]
    fn non_matching_value() {
        let mut tree = TreeInner::new();
        let expr = FilterExpressionInner::String {
            key: vec!["field".into()],
            expected: "other".into(),
        };
        tree.add_rule(&[expr], 1);
        let event = json!({"field": "value"});
        assert!(tree.get_matching_rules(&event).is_empty());
    }

    #[test]
    fn multi_segment_and_rule() {
        let mut tree = TreeInner::new();
        let expr1 = FilterExpressionInner::String {
            key: vec!["a".into()],
            expected: "1".into(),
        };
        let expr2 = FilterExpressionInner::String {
            key: vec!["b".into()],
            expected: "2".into(),
        };
        tree.add_rule(&[expr1, expr2], 42);
        let event = json!({"a": "1", "b": "2"});
        assert_eq!(tree.get_matching_rules(&event), vec![42]);
    }

    #[test]
    fn partial_and_does_not_match() {
        let mut tree = TreeInner::new();
        let expr1 = FilterExpressionInner::String {
            key: vec!["a".into()],
            expected: "1".into(),
        };
        let expr2 = FilterExpressionInner::String {
            key: vec!["b".into()],
            expected: "2".into(),
        };
        tree.add_rule(&[expr1, expr2], 42);
        let event = json!({"a": "1", "b": "WRONG"});
        assert!(tree.get_matching_rules(&event).is_empty());
    }

    #[test]
    fn deduplicates_rule_ids() {
        let mut tree = TreeInner::new();
        let expr = FilterExpressionInner::String {
            key: vec!["field".into()],
            expected: "val".into(),
        };
        tree.add_rule(&[expr.clone()], 1);
        tree.add_rule(&[expr], 1);
        let event = json!({"field": "val"});
        let result = tree.get_matching_rules(&event);
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn multiple_rules_match() {
        let mut tree = TreeInner::new();
        let expr1 = FilterExpressionInner::Exists {
            key: vec!["a".into()],
        };
        let expr2 = FilterExpressionInner::Exists {
            key: vec!["b".into()],
        };
        tree.add_rule(&[expr1.clone()], 10);
        tree.add_rule(&[expr1, expr2], 20);
        let event = json!({"a": 1, "b": 2});
        let matches = tree.get_matching_rules(&event);
        assert!(matches.contains(&10));
        assert!(matches.contains(&20));
    }

    #[test]
    fn child_lookup_respects_equality() {
        let mut tree = TreeInner::new();
        let expr_a = FilterExpressionInner::String {
            key: vec!["x".into()],
            expected: "1".into(),
        };
        let expr_b = FilterExpressionInner::String {
            key: vec!["x".into()],
            expected: "2".into(),
        };
        tree.add_rule(&[expr_a.clone()], 1);
        tree.add_rule(&[expr_a, expr_b], 2);
        assert_eq!(tree.root().children.len(), 1);
    }

    #[test]
    fn size_counts_all_nodes() {
        let mut tree = TreeInner::new();
        tree.add_rule(
            &[FilterExpressionInner::Exists {
                key: vec!["a".into()],
            }],
            1,
        );
        tree.add_rule(
            &[
                FilterExpressionInner::Exists {
                    key: vec!["a".into()],
                },
                FilterExpressionInner::Exists {
                    key: vec!["b".into()],
                },
            ],
            2,
        );
        assert_eq!(tree.size(), 3);
    }
}
