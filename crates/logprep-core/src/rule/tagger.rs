use std::collections::HashMap;

use crate::filter::expression::FilterExpressionInner;

pub struct RuleTaggerInner;

impl RuleTaggerInner {
    pub fn add_tags(
        segments: &mut Vec<Vec<FilterExpressionInner>>,
        tag_map: &HashMap<String, String>,
    ) {
        if tag_map.is_empty() {
            return;
        }

        for segment in segments.iter_mut() {
            Self::add_tags_to_segment(segment, tag_map);
        }
    }

    fn add_tags_to_segment(
        segment: &mut Vec<FilterExpressionInner>,
        tag_map: &HashMap<String, String>,
    ) {
        let mut tags_to_add: Vec<FilterExpressionInner> = Vec::new();

        for expr in segment.iter() {
            let inner = match expr {
                FilterExpressionInner::Not { child } => child.as_ref(),
                other => other,
            };

            if let Some(key) = Self::expression_key(inner) {
                if let Some(tag_value) = tag_map.get(&key[0]) {
                    let tag_expr = if tag_value.contains(':') {
                        let parts: Vec<&str> = tag_value.splitn(2, ':').collect();
                        let tag_key = parts[0].split('.').map(String::from).collect::<Vec<_>>();
                        FilterExpressionInner::String {
                            key: tag_key,
                            expected: parts[1].to_string(),
                        }
                    } else {
                        FilterExpressionInner::Exists {
                            key: tag_value.split('.').map(String::from).collect::<Vec<_>>(),
                        }
                    };
                    if !segment.contains(&tag_expr) {
                        tags_to_add.push(tag_expr);
                    }
                }
            }
        }

        for tag in tags_to_add.into_iter().rev() {
            segment.insert(0, tag);
        }
    }

    fn expression_key(expr: &FilterExpressionInner) -> Option<&Vec<String>> {
        match expr {
            FilterExpressionInner::String { key, .. }
            | FilterExpressionInner::Wildcard { key, .. }
            | FilterExpressionInner::Sigma { key, .. }
            | FilterExpressionInner::Integer { key, .. }
            | FilterExpressionInner::Float { key, .. }
            | FilterExpressionInner::IntegerRange { key, .. }
            | FilterExpressionInner::FloatRange { key, .. }
            | FilterExpressionInner::StringRange { key, .. }
            | FilterExpressionInner::Regex { key, .. }
            | FilterExpressionInner::Exists { key }
            | FilterExpressionInner::Null { key } => Some(key),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_tag_to_matching_segment() {
        let mut segments = vec![vec![FilterExpressionInner::String {
            key: vec!["field".into()],
            expected: "val".into(),
        }]];
        let mut tag_map = HashMap::new();
        tag_map.insert("field".into(), "check-tag".into());

        RuleTaggerInner::add_tags(&mut segments, &tag_map);

        assert_eq!(segments[0].len(), 2);
        assert!(matches!(
            &segments[0][0],
            FilterExpressionInner::Exists { key } if key == &vec!["check-tag".to_string()]
        ));
    }

    #[test]
    fn no_tag_map_no_change() {
        let mut segments = vec![vec![FilterExpressionInner::Exists {
            key: vec!["a".into()],
        }]];
        RuleTaggerInner::add_tags(&mut segments, &HashMap::new());
        assert_eq!(segments[0].len(), 1);
    }
}
