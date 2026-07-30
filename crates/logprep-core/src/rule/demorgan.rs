use crate::filter::expression::FilterExpressionInner;

pub struct DeMorganResolverInner;

impl DeMorganResolverInner {
    pub fn resolve(expr: &FilterExpressionInner) -> FilterExpressionInner {
        match expr {
            FilterExpressionInner::Not { child } => Self::resolve_not(child),
            FilterExpressionInner::And { children } => {
                let resolved: Vec<_> = children.iter().map(Self::resolve).collect();
                FilterExpressionInner::And {
                    children: resolved,
                }
            }
            FilterExpressionInner::Or { children } => {
                let resolved: Vec<_> = children.iter().map(Self::resolve).collect();
                FilterExpressionInner::Or {
                    children: resolved,
                }
            }
            other => other.clone(),
        }
    }

    fn resolve_not(inner: &FilterExpressionInner) -> FilterExpressionInner {
        match inner {
            FilterExpressionInner::Not { child } => Self::resolve(child),
            FilterExpressionInner::And { children } => {
                let negated: Vec<_> = children
                    .iter()
                    .map(|c| {
                        Self::resolve(&FilterExpressionInner::Not {
                            child: Box::new(c.clone()),
                        })
                    })
                    .collect();
                FilterExpressionInner::Or {
                    children: negated,
                }
            }
            FilterExpressionInner::Or { children } => {
                let negated: Vec<_> = children
                    .iter()
                    .map(|c| {
                        Self::resolve(&FilterExpressionInner::Not {
                            child: Box::new(c.clone()),
                        })
                    })
                    .collect();
                FilterExpressionInner::And {
                    children: negated,
                }
            }
            other => FilterExpressionInner::Not {
                child: Box::new(Self::resolve(other)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_not_stays() {
        let expr = FilterExpressionInner::Not {
            child: Box::new(FilterExpressionInner::Exists {
                key: vec!["a".into()],
            }),
        };
        let resolved = DeMorganResolverInner::resolve(&expr);
        assert!(matches!(resolved, FilterExpressionInner::Not { .. }));
    }

    #[test]
    fn not_and_becomes_or() {
        let expr = FilterExpressionInner::Not {
            child: Box::new(FilterExpressionInner::And {
                children: vec![
                    FilterExpressionInner::Exists {
                        key: vec!["a".into()],
                    },
                    FilterExpressionInner::Exists {
                        key: vec!["b".into()],
                    },
                ],
            }),
        };
        let resolved = DeMorganResolverInner::resolve(&expr);
        assert!(matches!(resolved, FilterExpressionInner::Or { .. }));
    }

    #[test]
    fn not_or_becomes_and() {
        let expr = FilterExpressionInner::Not {
            child: Box::new(FilterExpressionInner::Or {
                children: vec![
                    FilterExpressionInner::Exists {
                        key: vec!["a".into()],
                    },
                    FilterExpressionInner::Exists {
                        key: vec!["b".into()],
                    },
                ],
            }),
        };
        let resolved = DeMorganResolverInner::resolve(&expr);
        assert!(matches!(resolved, FilterExpressionInner::And { .. }));
    }

    #[test]
    fn double_not_cancels() {
        let expr = FilterExpressionInner::Not {
            child: Box::new(FilterExpressionInner::Not {
                child: Box::new(FilterExpressionInner::Exists {
                    key: vec!["a".into()],
                }),
            }),
        };
        let resolved = DeMorganResolverInner::resolve(&expr);
        assert!(matches!(resolved, FilterExpressionInner::Exists { .. }));
    }

    #[test]
    fn non_not_unchanged() {
        let expr = FilterExpressionInner::Always { value: true };
        let resolved = DeMorganResolverInner::resolve(&expr);
        assert!(matches!(
            resolved,
            FilterExpressionInner::Always { value: true }
        ));
    }
}
