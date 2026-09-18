use std::collections::HashMap;

use crate::filter::expression::FilterExpressionInner;

pub struct RuleSorterInner;

impl RuleSorterInner {
    pub fn sort_segments(
        segments: &mut [Vec<FilterExpressionInner>],
        priority_dict: &HashMap<String, String>,
    ) {
        let mut key_cache: HashMap<String, Option<String>> = HashMap::new();

        for segment in segments.iter_mut() {
            segment.sort_by(|a, b| {
                let key_a = Self::sorting_key(a, priority_dict, &mut key_cache);
                let key_b = Self::sorting_key(b, priority_dict, &mut key_cache);
                key_a.cmp(&key_b)
            });
        }
    }

    fn sorting_key(
        expr: &FilterExpressionInner,
        priority_dict: &HashMap<String, String>,
        cache: &mut HashMap<String, Option<String>>,
    ) -> Option<String> {
        if matches!(expr, FilterExpressionInner::Always { .. }) {
            return None;
        }

        let dotted = match expr {
            FilterExpressionInner::Not { child } => {
                return Self::sorting_key(child, priority_dict, cache);
            }
            FilterExpressionInner::String { key, .. }
            | FilterExpressionInner::Wildcard { key, .. }
            | FilterExpressionInner::Sigma { key, .. }
            | FilterExpressionInner::Integer { key, .. }
            | FilterExpressionInner::Float { key, .. }
            | FilterExpressionInner::IntegerRange { key, .. }
            | FilterExpressionInner::FloatRange { key, .. }
            | FilterExpressionInner::NumericRange { key, .. }
            | FilterExpressionInner::StringRange { key, .. }
            | FilterExpressionInner::Regex { key, .. }
            | FilterExpressionInner::Exists { key }
            | FilterExpressionInner::Null { key } => key.join("."),
            _ => return Some(expr.to_repr()),
        };

        if let Some(priority) = priority_dict.get(&dotted) {
            Some(priority.clone())
        } else {
            let repr = expr.to_repr();
            if let Some(cached) = cache.get(&repr) {
                cached.clone()
            } else {
                cache.insert(repr.clone(), Some(repr.clone()));
                Some(repr)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_by_priority_within_segment() {
        let mut segments = vec![vec![
            FilterExpressionInner::String {
                key: vec!["z".into()],
                expected: "1".into(),
            },
            FilterExpressionInner::String {
                key: vec!["a".into()],
                expected: "2".into(),
            },
        ]];
        let mut priority = HashMap::new();
        priority.insert("a".into(), "01".into());
        RuleSorterInner::sort_segments(&mut segments, &priority);
        let first_key = match &segments[0][0] {
            FilterExpressionInner::String { key, .. } => key[0].clone(),
            _ => panic!(),
        };
        assert_eq!(first_key, "a");
    }

    #[test]
    fn always_first() {
        let mut segments = vec![
            vec![FilterExpressionInner::Always { value: true }],
            vec![FilterExpressionInner::Exists {
                key: vec!["a".into()],
            }],
        ];
        RuleSorterInner::sort_segments(&mut segments, &HashMap::new());
        assert!(matches!(
            segments[0][0],
            FilterExpressionInner::Always { .. }
        ));
    }
}
