//! Compiles normalized dependency expressions into PubGrub clauses.

use pubgrub::{IncompatibilityConstraint, IncompatibilityConstraintTerm, Ranges};

use crate::loader::LoaderKind;
use crate::metadata::{DependencyExpression, DependencyKind, Environment, ModDependency};
use crate::resolver::provider::PackageIncompatibilities;
use crate::resolver::types::{SolverPackage, SolverVersion};

use super::graph::{ExclusionMap, dependency_constraint, is_excluded, logical_package};

pub(super) fn compile_dependency_constraints(
    expressions: &[DependencyExpression],
    package: &str,
    loader: LoaderKind,
    exclusions: &ExclusionMap,
    target: Environment,
) -> PackageIncompatibilities {
    let mut output = Vec::new();
    for expression in expressions {
        if let Some(required) = required_formula(expression, package, loader, exclusions, target) {
            for clause in to_cnf(required) {
                output.push(IncompatibilityConstraint {
                    terms: clause.into_iter().map(Literal::negated_term).collect(),
                    reason: format!(
                        "{package} requires at least one compatible dependency in {}",
                        describe_expression(expression)
                    ),
                });
            }
        }
        if let Some(incompatible) = kind_formula(
            expression,
            DependencyKind::Incompatible,
            package,
            loader,
            exclusions,
            target,
        ) {
            append_forbidden_formula(
                &mut output,
                incompatible,
                format!(
                    "{package} is incompatible with {}",
                    describe_expression(expression)
                ),
            );
        }
        for relation in expression.relations() {
            if !relation.environment.applies_to(target)
                || is_excluded(exclusions, package, &relation.id)
            {
                continue;
            }
            match relation.kind {
                DependencyKind::Optional => {
                    let allowed =
                        dependency_constraint(&relation.id, &relation.requirement, loader);
                    let mut bad = Formula::Atom {
                        package: logical_package(&relation.id),
                        versions: Box::new(allowed.complement()),
                    };
                    if let Some(unless) = &relation.unless {
                        bad = Formula::And(vec![
                            bad,
                            Formula::Not(Box::new(presence_formula(unless, loader))),
                        ]);
                    }
                    append_forbidden_formula(
                        &mut output,
                        bad,
                        relation_reason(package, relation, "is optional but incompatible"),
                    );
                }
                DependencyKind::Required
                | DependencyKind::Recommended
                | DependencyKind::Suggested
                | DependencyKind::Incompatible
                | DependencyKind::Discouraged => {}
            }
        }
    }
    output
}

fn kind_formula(
    expression: &DependencyExpression,
    kind: DependencyKind,
    package: &str,
    loader: LoaderKind,
    exclusions: &ExclusionMap,
    target: Environment,
) -> Option<Formula> {
    match expression {
        DependencyExpression::Only(relation)
            if relation.kind == kind
                && relation.environment.applies_to(target)
                && !is_excluded(exclusions, package, &relation.id) =>
        {
            let atom = Formula::Atom {
                package: logical_package(&relation.id),
                versions: Box::new(dependency_constraint(
                    &relation.id,
                    &relation.requirement,
                    loader,
                )),
            };
            Some(match &relation.unless {
                Some(unless) => Formula::And(vec![
                    atom,
                    Formula::Not(Box::new(presence_formula(unless, loader))),
                ]),
                None => atom,
            })
        }
        DependencyExpression::Only(_) => None,
        DependencyExpression::Any(expressions) => {
            let expressions: Vec<_> = expressions
                .iter()
                .filter_map(|expression| {
                    kind_formula(expression, kind, package, loader, exclusions, target)
                })
                .collect();
            (!expressions.is_empty()).then_some(Formula::Or(expressions))
        }
        DependencyExpression::All(expressions) => {
            let expressions: Vec<_> = expressions
                .iter()
                .filter_map(|expression| {
                    kind_formula(expression, kind, package, loader, exclusions, target)
                })
                .collect();
            (!expressions.is_empty()).then_some(Formula::And(expressions))
        }
    }
}

fn append_forbidden_formula(
    output: &mut PackageIncompatibilities,
    formula: Formula,
    reason: String,
) {
    for conjunction in to_dnf(formula) {
        output.push(IncompatibilityConstraint {
            terms: conjunction.into_iter().map(Literal::term).collect(),
            reason: reason.clone(),
        });
    }
}

fn required_formula(
    expression: &DependencyExpression,
    package: &str,
    loader: LoaderKind,
    exclusions: &ExclusionMap,
    target: Environment,
) -> Option<Formula> {
    match expression {
        DependencyExpression::Only(relation)
            if relation.kind == DependencyKind::Required
                && relation.environment.applies_to(target)
                && !is_excluded(exclusions, package, &relation.id) =>
        {
            let atom = Formula::Atom {
                package: logical_package(&relation.id),
                versions: Box::new(dependency_constraint(
                    &relation.id,
                    &relation.requirement,
                    loader,
                )),
            };
            Some(match &relation.unless {
                Some(unless) => Formula::Or(vec![atom, presence_formula(unless, loader)]),
                None => atom,
            })
        }
        DependencyExpression::Only(_) => None,
        DependencyExpression::Any(expressions) => {
            let expressions: Vec<_> = expressions
                .iter()
                .filter_map(|expression| {
                    required_formula(expression, package, loader, exclusions, target)
                })
                .collect();
            (!expressions.is_empty()).then_some(Formula::Or(expressions))
        }
        DependencyExpression::All(expressions) => {
            let expressions: Vec<_> = expressions
                .iter()
                .filter_map(|expression| {
                    required_formula(expression, package, loader, exclusions, target)
                })
                .collect();
            (!expressions.is_empty()).then_some(Formula::And(expressions))
        }
    }
}

fn presence_formula(expression: &DependencyExpression, loader: LoaderKind) -> Formula {
    match expression {
        DependencyExpression::Only(relation) => {
            let atom = Formula::Atom {
                package: logical_package(&relation.id),
                versions: Box::new(dependency_constraint(
                    &relation.id,
                    &relation.requirement,
                    loader,
                )),
            };
            match &relation.unless {
                Some(unless) => Formula::Or(vec![atom, presence_formula(unless, loader)]),
                None => atom,
            }
        }
        DependencyExpression::Any(expressions) => Formula::Or(
            expressions
                .iter()
                .map(|expression| presence_formula(expression, loader))
                .collect(),
        ),
        DependencyExpression::All(expressions) => Formula::And(
            expressions
                .iter()
                .map(|expression| presence_formula(expression, loader))
                .collect(),
        ),
    }
}

#[derive(Clone)]
enum Formula {
    True,
    False,
    Atom {
        package: SolverPackage,
        versions: Box<Ranges<SolverVersion>>,
    },
    Not(Box<Formula>),
    And(Vec<Formula>),
    Or(Vec<Formula>),
}

#[derive(Clone)]
enum Literal {
    Positive(SolverPackage, Ranges<SolverVersion>),
    Negative(SolverPackage, Ranges<SolverVersion>),
}

impl Literal {
    fn term(self) -> IncompatibilityConstraintTerm<SolverPackage, Ranges<SolverVersion>> {
        match self {
            Self::Positive(package, versions) => {
                IncompatibilityConstraintTerm::Positive(package, versions)
            }
            Self::Negative(package, versions) => {
                IncompatibilityConstraintTerm::Negative(package, versions)
            }
        }
    }

    fn negated_term(self) -> IncompatibilityConstraintTerm<SolverPackage, Ranges<SolverVersion>> {
        match self {
            Self::Positive(package, versions) => {
                IncompatibilityConstraintTerm::Negative(package, versions)
            }
            Self::Negative(package, versions) => {
                IncompatibilityConstraintTerm::Positive(package, versions)
            }
        }
    }
}

fn to_cnf(formula: Formula) -> Vec<Vec<Literal>> {
    match into_nnf(formula) {
        Formula::True => Vec::new(),
        Formula::False => vec![Vec::new()],
        Formula::Atom { package, versions } => {
            vec![vec![Literal::Positive(package, *versions)]]
        }
        Formula::Not(formula) => match *formula {
            Formula::Atom { package, versions } => {
                vec![vec![Literal::Negative(package, *versions)]]
            }
            _ => unreachable!("NNF contains Not only around atoms"),
        },
        Formula::And(formulas) => formulas.into_iter().flat_map(to_cnf).collect(),
        Formula::Or(formulas) => {
            let mut result = vec![Vec::new()];
            for formula in formulas {
                let clauses = to_cnf(formula);
                if clauses.is_empty() {
                    return Vec::new();
                }
                result = result
                    .into_iter()
                    .flat_map(|left| {
                        clauses.iter().cloned().map(move |mut right| {
                            let mut clause = left.clone();
                            clause.append(&mut right);
                            clause
                        })
                    })
                    .collect();
            }
            result
        }
    }
}

fn to_dnf(formula: Formula) -> Vec<Vec<Literal>> {
    match into_nnf(formula) {
        Formula::True => vec![Vec::new()],
        Formula::False => Vec::new(),
        Formula::Atom { package, versions } => {
            vec![vec![Literal::Positive(package, *versions)]]
        }
        Formula::Not(formula) => match *formula {
            Formula::Atom { package, versions } => {
                vec![vec![Literal::Negative(package, *versions)]]
            }
            _ => unreachable!("NNF contains Not only around atoms"),
        },
        Formula::Or(formulas) => formulas.into_iter().flat_map(to_dnf).collect(),
        Formula::And(formulas) => {
            let mut result = vec![Vec::new()];
            for formula in formulas {
                let conjunctions = to_dnf(formula);
                if conjunctions.is_empty() {
                    return Vec::new();
                }
                result = result
                    .into_iter()
                    .flat_map(|left| {
                        conjunctions.iter().cloned().map(move |mut right| {
                            let mut conjunction = left.clone();
                            conjunction.append(&mut right);
                            conjunction
                        })
                    })
                    .collect();
            }
            result
        }
    }
}

fn into_nnf(formula: Formula) -> Formula {
    match formula {
        Formula::Not(formula) => match *formula {
            Formula::True => Formula::False,
            Formula::False => Formula::True,
            Formula::Atom { package, versions } => {
                Formula::Not(Box::new(Formula::Atom { package, versions }))
            }
            Formula::Not(formula) => into_nnf(*formula),
            Formula::And(formulas) => Formula::Or(
                formulas
                    .into_iter()
                    .map(|formula| into_nnf(Formula::Not(Box::new(formula))))
                    .collect(),
            ),
            Formula::Or(formulas) => Formula::And(
                formulas
                    .into_iter()
                    .map(|formula| into_nnf(Formula::Not(Box::new(formula))))
                    .collect(),
            ),
        },
        Formula::And(formulas) => Formula::And(formulas.into_iter().map(into_nnf).collect()),
        Formula::Or(formulas) => Formula::Or(formulas.into_iter().map(into_nnf).collect()),
        other => other,
    }
}

pub(super) fn relation_reason(package: &str, dependency: &ModDependency, relation: &str) -> String {
    let reason = dependency
        .reason
        .as_deref()
        .map(|reason| format!(": {reason}"))
        .unwrap_or_default();
    format!(
        "{package} {relation} {} {}{reason}",
        dependency.id, dependency.requirement
    )
}

fn describe_expression(expression: &DependencyExpression) -> String {
    expression
        .relations()
        .into_iter()
        .map(|dependency| format!("{} {}", dependency.id, dependency.requirement))
        .collect::<Vec<_>>()
        .join(" / ")
}
