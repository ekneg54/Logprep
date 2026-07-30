use std::collections::HashMap;

use crate::filter::expression::FilterExpressionInner;

use super::demorgan::DeMorganResolverInner;
use super::segmenter::RuleSegmenterInner;
use super::sorter::RuleSorterInner;
use super::tagger::RuleTaggerInner;

pub struct RuleParserInner;

impl RuleParserInner {
    pub fn parse(
        expr: &FilterExpressionInner,
        priority_dict: &HashMap<String, String>,
        tag_map: &HashMap<String, String>,
    ) -> Vec<Vec<FilterExpressionInner>> {
        let resolved = DeMorganResolverInner::resolve(expr);

        let mut segments = RuleSegmenterInner::segment_into_dnf(&resolved);

        RuleSorterInner::sort_segments(&mut segments, priority_dict);

        Self::add_exists_filters(&mut segments);

        RuleTaggerInner::add_tags(&mut segments, tag_map);

        segments
    }

    fn add_exists_filters(segments: &mut Vec<Vec<FilterExpressionInner>>) {
        for segment in segments.iter_mut() {
            let mut i = 0;
            let mut added = 0;
            let original_len = segment.len();
            while i < original_len {
                let expr = &segment[i + added];
                match expr {
                    FilterExpressionInner::Exists { .. }
                    | FilterExpressionInner::Not { .. }
                    | FilterExpressionInner::Always { .. } => {
                        i += 1;
                        continue;
                    }
                    _ => {}
                }
                let key = Self::extract_key(expr);
                if let Some(k) = key {
                    let exists = FilterExpressionInner::Exists { key: k.clone() };
                    if !segment[..i + added].contains(&exists) {
                        segment.insert(i + added, exists);
                        added += 1;
                    }
                }
                i += 1;
            }
        }
    }

    fn extract_key(expr: &FilterExpressionInner) -> Option<&Vec<String>> {
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
            | FilterExpressionInner::Null { key } => Some(key),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_pipeline_simple_string() {
        let expr = FilterExpressionInner::String {
            key: vec!["field".into()],
            expected: "val".into(),
        };
        let priority = HashMap::new();
        let tag_map = HashMap::new();
        let result = RuleParserInner::parse(&expr, &priority, &tag_map);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 2);
        assert!(matches!(&result[0][0], FilterExpressionInner::Exists { .. }));
        assert!(matches!(&result[0][1], FilterExpressionInner::String { .. }));
    }

    #[test]
    fn full_pipeline_or() {
        let expr = FilterExpressionInner::Or {
            children: vec![
                FilterExpressionInner::String {
                    key: vec!["a".into()],
                    expected: "1".into(),
                },
                FilterExpressionInner::String {
                    key: vec!["b".into()],
                    expected: "2".into(),
                },
            ],
        };
        let priority = HashMap::new();
        let tag_map = HashMap::new();
        let result = RuleParserInner::parse(&expr, &priority, &tag_map);
        assert_eq!(result.len(), 2);
        for segment in &result {
            assert_eq!(segment.len(), 2);
            assert!(matches!(&segment[0], FilterExpressionInner::Exists { .. }));
        }
    }
}
