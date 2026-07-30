use crate::filter::expression::FilterExpressionInner;

pub struct RuleSegmenterInner;

impl RuleSegmenterInner {
    pub fn segment_into_dnf(expr: &FilterExpressionInner) -> Vec<Vec<FilterExpressionInner>> {
        if Self::has_disjunction(expr) {
            Self::segment_expression(expr)
        } else if matches!(expr, FilterExpressionInner::And { .. }) {
            vec![Self::segment_conjunctive(expr)]
        } else {
            vec![vec![expr.clone()]]
        }
    }

    fn has_disjunction(expr: &FilterExpressionInner) -> bool {
        match expr {
            FilterExpressionInner::Or { .. } => true,
            FilterExpressionInner::And { children } => {
                children.iter().any(|c| Self::has_disjunction(c))
            }
            FilterExpressionInner::Not { child } => Self::has_disjunction(child),
            _ => false,
        }
    }

    fn segment_expression(expr: &FilterExpressionInner) -> Vec<Vec<FilterExpressionInner>> {
        if !Self::has_disjunction(expr) {
            if matches!(expr, FilterExpressionInner::And { .. }) {
                return vec![Self::segment_conjunctive(expr)];
            }
            return vec![vec![expr.clone()]];
        }
        match expr {
            FilterExpressionInner::Or { children } => Self::segment_disjunctive(children),
            FilterExpressionInner::And { children } => {
                let segmented: Vec<_> = children
                    .iter()
                    .map(|c| Self::segment_expression(c))
                    .collect();
                let mut cnf_clauses: Vec<FilterExpressionInner> = Vec::new();
                for seg in &segmented {
                    if seg.len() == 1 && seg[0].len() == 1 {
                        cnf_clauses.push(seg[0][0].clone());
                    } else if seg.len() == 1 {
                        let and_children: Vec<_> = seg[0].iter().map(|e| {
                            if matches!(e, FilterExpressionInner::And { .. }) {
                                e.clone()
                            } else { e.clone() }
                        }).collect();
                        cnf_clauses.push(FilterExpressionInner::And { children: and_children });
                    } else {
                        let or_children: Vec<_> = seg.iter().map(|branch| {
                            if branch.len() == 1 {
                                branch[0].clone()
                            } else {
                                FilterExpressionInner::And { children: branch.clone() }
                            }
                        }).collect();
                        cnf_clauses.push(FilterExpressionInner::Or { children: or_children });
                    }
                }
                CnfToDnfConverterInner::convert(&cnf_clauses)
            }
            _ => vec![vec![expr.clone()]],
        }
    }

    fn segment_disjunctive(children: &[FilterExpressionInner]) -> Vec<Vec<FilterExpressionInner>> {
        let mut result = Vec::new();
        for child in children {
            let segmented = Self::segment_expression(child);
            for seg in segmented {
                if seg.len() == 1 {
                    result.push(vec![seg.into_iter().next().unwrap()]);
                } else {
                    result.push(seg);
                }
            }
        }
        result
    }

    fn segment_conjunctive(expr: &FilterExpressionInner) -> Vec<FilterExpressionInner> {
        match expr {
            FilterExpressionInner::And { children } => {
                let mut result = Vec::new();
                for child in children {
                    if matches!(child, FilterExpressionInner::And { .. }) {
                        result.extend(Self::segment_conjunctive(child));
                    } else {
                        result.push(child.clone());
                    }
                }
                result
            }
            other => vec![other.clone()],
        }
    }
}

pub struct CnfToDnfConverterInner;

impl CnfToDnfConverterInner {
    pub fn convert(cnf: &[FilterExpressionInner]) -> Vec<Vec<FilterExpressionInner>> {
        let mut non_or: Vec<FilterExpressionInner> = Vec::new();
        let mut or_segments: Vec<Vec<FilterExpressionInner>> = Vec::new();

        for item in cnf {
            if let FilterExpressionInner::Or { children } = item {
                or_segments.push(children.clone());
            } else {
                non_or.push(item.clone());
            }
        }

        if or_segments.is_empty() {
            return vec![cnf.to_vec()];
        }

        let first_or = &or_segments[0];
        let mut dnf = Vec::new();

        for or_elem in first_or {
            let mut and_group = Vec::new();
            and_group.push(or_elem.clone());
            and_group.extend(non_or.clone());
            for remaining in &or_segments[1..] {
                for rem_elem in remaining {
                    let mut extended = and_group.clone();
                    extended.push(rem_elem.clone());
                    dnf.push(extended);
                }
            }
            if or_segments.len() == 1 {
                dnf.push(and_group);
            }
        }

        dnf.sort_by(|a, b| {
            let a_repr: Vec<String> = a.iter().map(|e| e.to_repr()).collect();
            let b_repr: Vec<String> = b.iter().map(|e| e.to_repr()).collect();
            a_repr.join("|").cmp(&b_repr.join("|"))
        });
        dnf.dedup_by(|a, b| {
            let a_repr: Vec<String> = a.iter().map(|e| e.to_repr()).collect();
            let b_repr: Vec<String> = b.iter().map(|e| e.to_repr()).collect();
            a_repr.join("|") == b_repr.join("|")
        });
        dnf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exists(key: &str) -> FilterExpressionInner {
        FilterExpressionInner::Exists {
            key: vec![key.into()],
        }
    }

    fn string_expr(key: &str, val: &str) -> FilterExpressionInner {
        FilterExpressionInner::String {
            key: vec![key.into()],
            expected: val.into(),
        }
    }

    #[test]
    fn simple_expression_stays() {
        let expr = exists("a");
        let dnf = RuleSegmenterInner::segment_into_dnf(&expr);
        assert_eq!(dnf.len(), 1);
        assert_eq!(dnf[0].len(), 1);
    }

    #[test]
    fn and_expression() {
        let expr = FilterExpressionInner::And {
            children: vec![exists("a"), exists("b")],
        };
        let dnf = RuleSegmenterInner::segment_into_dnf(&expr);
        assert_eq!(dnf.len(), 1);
        assert_eq!(dnf[0].len(), 2);
    }

    #[test]
    fn or_expression() {
        let expr = FilterExpressionInner::Or {
            children: vec![exists("a"), exists("b")],
        };
        let dnf = RuleSegmenterInner::segment_into_dnf(&expr);
        assert_eq!(dnf.len(), 2);
        assert_eq!(dnf[0].len(), 1);
        assert_eq!(dnf[1].len(), 1);
    }

    #[test]
    fn distribution_a_or_b_and_c() {
        let expr = FilterExpressionInner::And {
            children: vec![
                FilterExpressionInner::Or {
                    children: vec![string_expr("a", "1"), string_expr("b", "2")],
                },
                string_expr("c", "3"),
            ],
        };
        let dnf = RuleSegmenterInner::segment_into_dnf(&expr);
        assert_eq!(dnf.len(), 2, "DNF should have 2 OR branches");
        for branch in &dnf {
            assert_eq!(branch.len(), 2, "Each branch should have 2 AND expressions");
            assert!(
                branch
                    .iter()
                    .any(|e| matches!(e, FilterExpressionInner::String { expected, .. } if expected == "3"))
            );
        }
    }
}
