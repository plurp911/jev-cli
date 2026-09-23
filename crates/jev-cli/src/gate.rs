//! `--require`: turning a model judgment into a process exit status, safely.
//!
//! # Why a language at all
//!
//! Because the alternative is worse. Without it, a CI job writes
//! `jev noul … --value | awk '{exit !($1 > 0.9)}'`, which silently passes when `jev`
//! fails and prints nothing. A first-class gate can distinguish "the model said no"
//! from "the gate could not be evaluated", and can refuse to pass in the second case.
//!
//! # Why it is this small
//!
//! The grammar is deliberately tiny and total:
//!
//! ```text
//! expr       := or
//! or         := and ('or' and)*
//! and        := unary ('and' unary)*
//! unary      := 'not' unary | '(' expr ')' | comparison
//! comparison := path operator literal
//! path       := ident ('.' ident)*
//! operator   := '>' | '>=' | '<' | '<=' | '==' | '!='
//! literal    := number | 'single quoted' | "double quoted" | bare-word
//! ```
//!
//! There is no arithmetic, no function call, no variable, no string concatenation, and
//! no way to name anything outside the response. Nothing is ever handed to a shell —
//! there is no `eval` anywhere in this project. The parser is recursive descent with an
//! explicit depth bound, so a pathological expression cannot exhaust the stack, and it
//! is property-tested and fuzzed for totality.
//!
//! # Paths
//!
//! A path addresses a field of one answer, by question id:
//!
//! | Path | Reads |
//! | ---- | ----- |
//! | `urgent.noul` | the Noul probability |
//! | `team.choice` | the selected option name |
//! | `team.confidence` | the Choice or Score confidence |
//! | `team.probabilities.billing` | one option's probability |
//! | `severity.score` | the Score value |
//! | `severity.probabilities.2` | one level's probability |
//!
//! A path that does not resolve is **not** false. It is
//! [`GateOutcome::Unevaluable`], which exits `6`.

use std::collections::BTreeMap;
use std::fmt;

use jev_core::{Answer, EvaluationResponse};

/// Deepest nesting of parentheses and `not` accepted.
///
/// Part of the documented grammar; see `docs/commands.md`.
///
/// A recursive-descent parser on unbounded input is a stack-exhaustion bug; this bound
/// is what makes the parser total.
pub const MAX_DEPTH: usize = 32;

/// Longest expression accepted, in characters.
///
/// Part of the documented grammar; see `docs/commands.md`.
pub const MAX_EXPRESSION_LEN: usize = 4096;

/// Reasons an expression could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    /// The expression was empty.
    #[error("the expression is empty")]
    Empty,
    /// The expression was longer than the accepted maximum.
    #[error("the expression is longer than {MAX_EXPRESSION_LEN} characters")]
    TooLong,
    /// Parentheses or `not` nested deeper than the accepted maximum.
    #[error("the expression nests deeper than {MAX_DEPTH} levels")]
    TooDeep,
    /// A character that cannot appear in an expression.
    #[error("unexpected character {found:?} at position {position}")]
    UnexpectedCharacter {
        /// The offending character.
        found: char,
        /// Its index, in characters.
        position: usize,
    },
    /// Something other than what the grammar allows here.
    #[error("expected {expected} at position {position}, found {found}")]
    Expected {
        /// What the grammar allows.
        expected: &'static str,
        /// What was there.
        found: String,
        /// Where.
        position: usize,
    },
    /// Input ran out mid-expression.
    #[error("the expression ends unexpectedly; expected {expected}")]
    UnexpectedEnd {
        /// What the grammar allows.
        expected: &'static str,
    },
    /// Text after a complete expression.
    #[error("unexpected trailing input at position {position}")]
    TrailingInput {
        /// Where the extra text starts.
        position: usize,
    },
    /// An unterminated quoted literal.
    #[error("unterminated quoted value starting at position {position}")]
    UnterminatedString {
        /// Where the quote opened.
        position: usize,
    },
}

/// A comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    /// `>`
    Greater,
    /// `>=`
    GreaterOrEqual,
    /// `<`
    Less,
    /// `<=`
    LessOrEqual,
    /// `==`
    Equal,
    /// `!=`
    NotEqual,
}

impl Operator {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Greater => ">",
            Self::GreaterOrEqual => ">=",
            Self::Less => "<",
            Self::LessOrEqual => "<=",
            Self::Equal => "==",
            Self::NotEqual => "!=",
        }
    }
}

/// The right-hand side of a comparison.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// A number, for probabilities, confidences, and scores.
    Number(f64),
    /// Text, for a selected Choice option.
    Text(String),
}

/// A parsed gate expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `a and b`
    And(Box<Expr>, Box<Expr>),
    /// `a or b`
    Or(Box<Expr>, Box<Expr>),
    /// `not a`
    Not(Box<Expr>),
    /// `path op literal`
    Comparison {
        /// The dotted path into the response.
        path: Vec<String>,
        /// The operator.
        operator: Operator,
        /// The value compared against.
        literal: Literal,
    },
}

/// The result of evaluating a gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// The gate held.
    Passed,
    /// The gate was evaluated and did not hold.
    Failed,
    /// The gate could not be evaluated. **Never** treated as a pass.
    Unevaluable {
        /// Why, naming the path that did not resolve.
        reason: String,
    },
}

impl GateOutcome {
    /// Whether the gate held.
    ///
    /// Only [`GateOutcome::Passed`] is a pass. An unevaluable gate is not, and the
    /// property tests below assert that directly, because it is the safety property the
    /// exit-code contract depends on.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "asserted by the gate property tests")
    )]
    #[must_use]
    pub const fn passed(&self) -> bool {
        matches!(self, Self::Passed)
    }
}

/// A value a path resolved to.
#[derive(Debug, Clone, PartialEq)]
enum Resolved {
    Number(f64),
    Text(String),
}

/// Parses an expression.
///
/// # Errors
///
/// Returns [`ParseError`] describing what is wrong and where.
pub fn parse(input: &str) -> Result<Expr, ParseError> {
    if input.chars().count() > MAX_EXPRESSION_LEN {
        return Err(ParseError::TooLong);
    }
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut parser = Parser {
        tokens: &tokens,
        position: 0,
        depth: 0,
    };
    let expr = parser.parse_or()?;
    if let Some(token) = parser.peek() {
        return Err(ParseError::TrailingInput {
            position: token.position,
        });
    }
    Ok(expr)
}

/// Evaluates a parsed expression against a response.
#[must_use]
pub fn evaluate(expr: &Expr, response: &EvaluationResponse) -> GateOutcome {
    match eval(expr, response) {
        Ok(true) => GateOutcome::Passed,
        Ok(false) => GateOutcome::Failed,
        Err(reason) => GateOutcome::Unevaluable { reason },
    }
}

fn eval(expr: &Expr, response: &EvaluationResponse) -> Result<bool, String> {
    match expr {
        // Deliberately *not* short-circuiting. `a and b` where `b` names a
        // non-existent path is a broken gate, and a broken gate must be reported even
        // when `a` alone would decide the outcome — otherwise a typo hides until the
        // day the other side flips.
        Expr::And(left, right) => {
            // Both sides are evaluated *and both are unwrapped* before combining.
            // Writing `Ok(left? && right?)` would let `&&` short-circuit the `?`, so a
            // broken right-hand side would go unreported whenever the left is false --
            // exactly the silent-gate failure this arm exists to prevent.
            let left = eval(left, response);
            let right = eval(right, response);
            let (left, right) = (left?, right?);
            Ok(left && right)
        }
        Expr::Or(left, right) => {
            let left = eval(left, response);
            let right = eval(right, response);
            let (left, right) = (left?, right?);
            Ok(left || right)
        }
        Expr::Not(inner) => eval(inner, response).map(|value| !value),
        Expr::Comparison {
            path,
            operator,
            literal,
        } => compare(path, *operator, literal, response),
    }
}

fn compare(
    path: &[String],
    operator: Operator,
    literal: &Literal,
    response: &EvaluationResponse,
) -> Result<bool, String> {
    let rendered = path.join(".");
    let resolved = resolve(path, response)
        .ok_or_else(|| format!("`{rendered}` does not name anything in the response"))?;

    match (&resolved, literal) {
        (Resolved::Number(left), Literal::Number(right)) => Ok(match operator {
            Operator::Greater => left > right,
            Operator::GreaterOrEqual => left >= right,
            Operator::Less => left < right,
            Operator::LessOrEqual => left <= right,
            // Exact, like the four comparisons above it.
            //
            // This used to be `(left - right).abs() < f64::EPSILON`. `f64::EPSILON` is
            // an *absolute* 2.22e-16, so as a tolerance it inverted across the range:
            // near zero it was enormous in relative terms, and above about 2 it was
            // smaller than one ULP and degenerated to exact equality anyway. The visible
            // effect was that `x == 0` and `x > 0` both held for a probability of 1e-17,
            // so `==` was not the complement of `!=` and meant two different things on a
            // Noul and on a Score. One operator cannot have two meanings that depend on
            // the magnitude of the value it is given.
            //
            // Exact is the rule that can be stated in one line of documentation, and it
            // makes `==`/`!=` complementary and consistent with `>=`/`<=`. Equality on a
            // model-produced float is fragile whichever rule is chosen; `docs/commands.md`
            // says so and points at range comparisons instead.
            #[expect(
                clippy::float_cmp,
                reason = "exactness is the point, and the lint's suggested remedy -- an error margin \
                          -- is the defect this replaced. `f64::EPSILON` as an absolute tolerance made \
                          `x == 0` and `x > 0` both true for 1e-17 and degenerated to exact equality \
                          above ~2, so one operator meant two things depending on the magnitude of its \
                          input. `expect` rather than `allow` so this fails if the arms stop needing \
                          it"
            )]
            Operator::Equal => left == right,
            #[expect(clippy::float_cmp, reason = "the complement of Equal, above")]
            Operator::NotEqual => left != right,
        }),
        (Resolved::Text(left), Literal::Text(right)) => match operator {
            Operator::Equal => Ok(left == right),
            Operator::NotEqual => Ok(left != right),
            _ => Err(format!(
                "`{rendered}` is text, so it can only be compared with == or !=, \
                 not {}",
                operator.as_str()
            )),
        },
        (Resolved::Number(_), Literal::Text(text)) => Err(format!(
            "`{rendered}` is a number, but it is compared with the text {text:?}"
        )),
        (Resolved::Text(_), Literal::Number(number)) => Err(format!(
            "`{rendered}` is text, but it is compared with the number {number}"
        )),
    }
}

/// Walks a dotted path into one answer.
fn resolve(path: &[String], response: &EvaluationResponse) -> Option<Resolved> {
    let (id, rest) = path.split_first()?;
    let answer = response.answer(id)?;
    match (answer, rest.len()) {
        (Answer::Noul { noul }, 1) if rest.first().is_some_and(|f| f == "noul") => {
            Some(Resolved::Number(noul.get()))
        }
        (Answer::Choice { choice, .. }, 1) if rest.first().is_some_and(|f| f == "choice") => {
            Some(Resolved::Text(choice.clone()))
        }
        (Answer::Score { score, .. }, 1) if rest.first().is_some_and(|f| f == "score") => {
            Some(Resolved::Number(*score))
        }
        (_, 1) if rest.first().is_some_and(|f| f == "confidence") => {
            answer.confidence().map(|c| Resolved::Number(c.get()))
        }
        (Answer::Choice { probabilities, .. }, 2)
            if rest.first().is_some_and(|f| f == "probabilities") =>
        {
            let key = rest.get(1)?;
            probabilities
                .iter()
                .find(|entry| &entry.key == key)
                .map(|entry| Resolved::Number(entry.probability.get()))
        }
        (Answer::Score { probabilities, .. }, 2)
            if rest.first().is_some_and(|f| f == "probabilities") =>
        {
            let key: u32 = rest.get(1)?.parse().ok()?;
            probabilities
                .iter()
                .find(|entry| entry.key == key)
                .map(|entry| Resolved::Number(entry.probability.get()))
        }
        _ => None,
    }
}

/// The field names a path may end in, for `--help` and error hints.
#[must_use]
pub fn addressable_fields() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("<id>.noul", "the yes-probability of a Noul answer"),
        ("<id>.choice", "the selected option of a Choice answer"),
        ("<id>.score", "the value of a Score answer"),
        (
            "<id>.confidence",
            "the confidence of a Choice or Score answer",
        ),
        (
            "<id>.probabilities.<option|level>",
            "one entry of a distribution",
        ),
    ])
}

// --- Lexer -----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokenKind,
    position: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Path(Vec<String>),
    Operator(Operator),
    Number(f64),
    Text(String),
    And,
    Or,
    Not,
    OpenParen,
    CloseParen,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(parts) => write!(f, "`{}`", parts.join(".")),
            Self::Operator(operator) => write!(f, "`{}`", operator.as_str()),
            Self::Number(value) => write!(f, "{value}"),
            Self::Text(value) => write!(f, "{value:?}"),
            Self::And => f.write_str("`and`"),
            Self::Or => f.write_str("`or`"),
            Self::Not => f.write_str("`not`"),
            Self::OpenParen => f.write_str("`(`"),
            Self::CloseParen => f.write_str("`)`"),
        }
    }
}

fn tokenize(input: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        let Some(&current) = chars.get(index) else {
            break;
        };
        let start = index;

        if current.is_whitespace() {
            index += 1;
            continue;
        }

        match current {
            '(' => {
                tokens.push(Token {
                    kind: TokenKind::OpenParen,
                    position: start,
                });
                index += 1;
            }
            ')' => {
                tokens.push(Token {
                    kind: TokenKind::CloseParen,
                    position: start,
                });
                index += 1;
            }
            '>' | '<' | '=' | '!' => {
                let (operator, width) = lex_operator(&chars, index)?;
                index += width;
                tokens.push(Token {
                    kind: TokenKind::Operator(operator),
                    position: start,
                });
            }
            '\'' | '"' => {
                let (value, next) = lex_quoted(&chars, index, current)?;
                index = next;
                tokens.push(Token {
                    kind: TokenKind::Text(value),
                    position: start,
                });
            }
            c if c.is_ascii_digit() || c == '-' || c == '+' => {
                let (value, next) = lex_number(&chars, index)?;
                index = next;
                tokens.push(Token {
                    kind: TokenKind::Number(value),
                    position: start,
                });
            }
            c if is_word_start(c) => {
                let (word, next) = lex_word(&chars, index);
                index = next;
                let kind = match word.as_str() {
                    "and" => TokenKind::And,
                    "or" => TokenKind::Or,
                    "not" => TokenKind::Not,
                    _ if word.contains('.') => {
                        let parts: Vec<String> = word.split('.').map(str::to_owned).collect();
                        if parts.iter().any(String::is_empty) {
                            return Err(ParseError::Expected {
                                expected: "a path such as `urgent.noul`",
                                found: word.clone(),
                                position: start,
                            });
                        }
                        TokenKind::Path(parts)
                    }
                    // A bare word after an operator is an unquoted option name, which
                    // is the common case: `team == billing`.
                    _ if matches!(
                        tokens.last().map(|token| &token.kind),
                        Some(TokenKind::Operator(_))
                    ) =>
                    {
                        TokenKind::Text(word.clone())
                    }
                    _ => TokenKind::Path(vec![word.clone()]),
                };
                tokens.push(Token {
                    kind,
                    position: start,
                });
            }
            other => {
                return Err(ParseError::UnexpectedCharacter {
                    found: other,
                    position: start,
                });
            }
        }
    }
    Ok(tokens)
}

/// Reads a comparison operator, returning it and how many characters it spans.
fn lex_operator(chars: &[char], index: usize) -> Result<(Operator, usize), ParseError> {
    let current = chars.get(index).copied().unwrap_or(' ');
    let next = chars.get(index + 1).copied();
    let found = match (current, next) {
        ('>', Some('=')) => (Operator::GreaterOrEqual, 2),
        ('<', Some('=')) => (Operator::LessOrEqual, 2),
        ('=', Some('=')) => (Operator::Equal, 2),
        ('!', Some('=')) => (Operator::NotEqual, 2),
        ('>', _) => (Operator::Greater, 1),
        ('<', _) => (Operator::Less, 1),
        // A single `=` is the classic slip; naming it beats "unexpected character".
        _ => {
            return Err(ParseError::Expected {
                expected: "a comparison operator (>, >=, <, <=, ==, !=)",
                found: format!("{current:?}"),
                position: index,
            });
        }
    };
    Ok(found)
}

/// Reads a quoted literal, returning its contents and the index after the close quote.
fn lex_quoted(
    chars: &[char],
    mut index: usize,
    quote: char,
) -> Result<(String, usize), ParseError> {
    let start = index;
    index += 1;
    let mut value = String::new();
    loop {
        match chars.get(index) {
            None => return Err(ParseError::UnterminatedString { position: start }),
            Some(&c) if c == quote => return Ok((value, index + 1)),
            Some(&c) => {
                value.push(c);
                index += 1;
            }
        }
    }
}

/// Reads a numeric literal, returning its value and the index after it.
fn lex_number(chars: &[char], mut index: usize) -> Result<(f64, usize), ParseError> {
    let start = index;
    let mut raw = String::new();
    while let Some(&c) = chars.get(index) {
        if c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E') {
            raw.push(c);
            index += 1;
        } else {
            break;
        }
    }
    let value: f64 = raw.parse().map_err(|_| ParseError::Expected {
        expected: "a number",
        found: raw.clone(),
        position: start,
    })?;
    // `"inf"` and `"NaN"` do not reach here -- they lex as words -- but `1e400` parses
    // to infinity, and an infinite threshold is never what anyone meant.
    if !value.is_finite() {
        return Err(ParseError::Expected {
            expected: "a finite number",
            found: raw,
            position: start,
        });
    }
    Ok((value, index))
}

/// Reads a bare word, returning it and the index after it.
fn lex_word(chars: &[char], mut index: usize) -> (String, usize) {
    let mut word = String::new();
    while let Some(&c) = chars.get(index) {
        if is_word_char(c) {
            word.push(c);
            index += 1;
        } else {
            break;
        }
    }
    (word, index)
}

fn is_word_start(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '-')
}

// --- Parser ----------------------------------------------------------------------

struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn advance(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.position);
        if token.is_some() {
            self.position += 1;
        }
        token
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Or)) {
            self.position += 1;
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        while matches!(self.peek().map(|t| &t.kind), Some(TokenKind::And)) {
            self.position += 1;
            let right = self.parse_unary()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ParseError::TooDeep);
        }
        let result = self.parse_unary_inner();
        self.depth -= 1;
        result
    }

    fn parse_unary_inner(&mut self) -> Result<Expr, ParseError> {
        match self.peek().map(|token| token.kind.clone()) {
            Some(TokenKind::Not) => {
                self.position += 1;
                Ok(Expr::Not(Box::new(self.parse_unary()?)))
            }
            Some(TokenKind::OpenParen) => {
                self.position += 1;
                let inner = self.parse_or()?;
                match self.advance().map(|token| &token.kind) {
                    Some(TokenKind::CloseParen) => Ok(inner),
                    Some(other) => Err(ParseError::Expected {
                        expected: "`)`",
                        found: other.to_string(),
                        position: self.position.saturating_sub(1),
                    }),
                    None => Err(ParseError::UnexpectedEnd { expected: "`)`" }),
                }
            }
            Some(_) => self.parse_comparison(),
            None => Err(ParseError::UnexpectedEnd {
                expected: "a comparison",
            }),
        }
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let position = self.position;
        let path = match self.advance().map(|token| token.kind.clone()) {
            Some(TokenKind::Path(parts)) => parts,
            // A comparison reads `path operator literal`, so a number on the left is
            // either the two sides written the wrong way round, or a question id that
            // starts with a digit -- legal as an id, since `QuestionId` accepts anything
            // non-blank and control-free, but not addressable, because a leading digit
            // lexes as a number. Both are worth naming; blaming the grammar sent the
            // user to read the one thing they cannot change. The number stays in
            // `found`, because for `1urgent.noul > 0.5` seeing `1` is the clue.
            Some(TokenKind::Number(value)) => {
                return Err(ParseError::Expected {
                    expected: "a path such as `urgent.noul` on the left of the comparison \
                               (a question id that starts with a digit cannot be \
                               addressed by a gate, so rename it)",
                    found: format!("the number {value}"),
                    position,
                });
            }
            Some(other) => {
                return Err(ParseError::Expected {
                    expected: "a path such as `urgent.noul`",
                    found: other.to_string(),
                    position,
                });
            }
            None => {
                return Err(ParseError::UnexpectedEnd {
                    expected: "a path such as `urgent.noul`",
                });
            }
        };

        let operator = match self.advance().map(|token| token.kind.clone()) {
            Some(TokenKind::Operator(operator)) => operator,
            Some(other) => {
                return Err(ParseError::Expected {
                    expected: "a comparison operator (>, >=, <, <=, ==, !=)",
                    found: other.to_string(),
                    position: self.position.saturating_sub(1),
                });
            }
            None => {
                return Err(ParseError::UnexpectedEnd {
                    expected: "a comparison operator",
                });
            }
        };

        let literal = match self.advance().map(|token| token.kind.clone()) {
            Some(TokenKind::Number(value)) => Literal::Number(value),
            Some(TokenKind::Text(value)) => Literal::Text(value),
            Some(other) => {
                return Err(ParseError::Expected {
                    expected: "a number or a quoted value",
                    found: other.to_string(),
                    position: self.position.saturating_sub(1),
                });
            }
            None => {
                return Err(ParseError::UnexpectedEnd {
                    expected: "a number or a quoted value",
                });
            }
        };

        Ok(Expr::Comparison {
            path,
            operator,
            literal,
        })
    }
}

#[cfg(test)]
mod tests {
    use jev_core::{Confidence, ModelId, Probability, QuestionId, Usage, Weighted};
    use proptest::prelude::*;

    use super::*;

    fn probability(value: f64) -> Probability {
        Probability::new(value).unwrap()
    }

    fn id(value: &str) -> QuestionId {
        QuestionId::new(value).unwrap()
    }

    // --- The length bound ---------------------------------------------------------

    /// The bound is on *characters*, not bytes, and the boundary is where an off-by-one
    /// lives. `a` repeated to exactly the limit is a valid chain of conjunctions.
    #[test]
    fn an_expression_of_exactly_the_maximum_length_is_accepted() {
        // `x` padded with spaces: still one identifier, exactly MAX_EXPRESSION_LEN long.
        let expression = format!("x{}", " ".repeat(MAX_EXPRESSION_LEN - 1));
        assert_eq!(expression.chars().count(), MAX_EXPRESSION_LEN);
        assert!(
            !matches!(parse(&expression), Err(ParseError::TooLong)),
            "an expression at exactly the limit was refused for length"
        );
    }

    #[test]
    fn an_expression_one_character_over_the_maximum_is_refused() {
        let expression = format!("x{}", " ".repeat(MAX_EXPRESSION_LEN));
        assert!(matches!(parse(&expression), Err(ParseError::TooLong)));
    }

    /// A multibyte expression must be measured in characters. Counting bytes would
    /// refuse an expression a quarter of the documented length.
    #[test]
    fn the_length_bound_counts_characters_not_bytes() {
        // Four bytes each, so this is 4 * MAX_EXPRESSION_LEN bytes but exactly
        // MAX_EXPRESSION_LEN characters.
        let expression = "\u{1f600}".repeat(MAX_EXPRESSION_LEN);
        assert_eq!(expression.chars().count(), MAX_EXPRESSION_LEN);
        assert!(expression.len() > MAX_EXPRESSION_LEN);
        assert!(
            !matches!(parse(&expression), Err(ParseError::TooLong)),
            "the bound counted bytes rather than characters"
        );
    }

    /// A response carrying one of each primitive, to exercise every path shape.
    fn response() -> EvaluationResponse {
        EvaluationResponse {
            model: ModelId::new("jev-1.13.0").unwrap(),
            answers: vec![
                (
                    id("urgent"),
                    Answer::Noul {
                        noul: probability(0.92),
                    },
                ),
                (
                    id("team"),
                    Answer::Choice {
                        choice: "billing".to_owned(),
                        probabilities: vec![
                            Weighted {
                                key: "billing".to_owned(),
                                probability: probability(0.8),
                            },
                            Weighted {
                                key: "technical".to_owned(),
                                probability: probability(0.2),
                            },
                        ],
                        confidence: Confidence::new(0.75).unwrap(),
                    },
                ),
                (
                    id("severity"),
                    Answer::Score {
                        score: 1.3,
                        legend: BTreeMap::new(),
                        probabilities: vec![
                            Weighted {
                                key: 0,
                                probability: probability(0.0),
                            },
                            Weighted {
                                key: 1,
                                probability: probability(0.7),
                            },
                            Weighted {
                                key: 2,
                                probability: probability(0.3),
                            },
                        ],
                        confidence: Confidence::new(0.54).unwrap(),
                    },
                ),
            ],
            usage: Usage::default(),
        }
    }

    fn gate(expression: &str) -> GateOutcome {
        let parsed = parse(expression).unwrap_or_else(|error| panic!("{expression:?}: {error}"));
        evaluate(&parsed, &response())
    }

    #[test]
    fn every_documented_path_shape_resolves() {
        assert_eq!(gate("urgent.noul > 0.9"), GateOutcome::Passed);
        assert_eq!(gate("team.choice == billing"), GateOutcome::Passed);
        assert_eq!(gate("team.confidence >= 0.75"), GateOutcome::Passed);
        assert_eq!(
            gate("team.probabilities.technical < 0.5"),
            GateOutcome::Passed
        );
        assert_eq!(gate("severity.score > 1"), GateOutcome::Passed);
        assert_eq!(gate("severity.probabilities.1 == 0.7"), GateOutcome::Passed);
        assert_eq!(gate("severity.confidence < 0.6"), GateOutcome::Passed);
    }

    #[test]
    fn a_failing_comparison_fails_rather_than_erroring() {
        assert_eq!(gate("urgent.noul < 0.5"), GateOutcome::Failed);
        assert_eq!(gate("team.choice == technical"), GateOutcome::Failed);
    }

    #[test]
    fn boolean_operators_work_and_bind_conventionally() {
        assert_eq!(
            gate("urgent.noul > 0.9 and team.choice == billing"),
            GateOutcome::Passed
        );
        assert_eq!(
            gate("urgent.noul < 0.1 or team.choice == billing"),
            GateOutcome::Passed
        );
        assert_eq!(gate("not urgent.noul < 0.5"), GateOutcome::Passed);
        // `and` binds tighter than `or`, as everywhere else.
        assert_eq!(
            gate("urgent.noul < 0.1 and team.choice == technical or severity.score > 1"),
            GateOutcome::Passed
        );
        assert_eq!(
            gate("urgent.noul < 0.1 and (team.choice == technical or severity.score > 1)"),
            GateOutcome::Failed
        );
    }

    #[test]
    fn quoted_values_allow_spaces_and_punctuation() {
        let expression = parse("team.choice == 'needs a human'").unwrap();
        assert!(matches!(
            expression,
            Expr::Comparison {
                literal: Literal::Text(ref text),
                ..
            } if text == "needs a human"
        ));
        assert!(parse(r#"team.choice == "double quoted""#).is_ok());
    }

    #[test]
    fn an_unknown_question_is_unevaluable_not_false() {
        // The property that makes this worth having at all. A typo must not look like
        // the model saying no.
        let outcome = gate("typo.noul > 0.5");
        assert!(matches!(outcome, GateOutcome::Unevaluable { .. }));
        assert!(!outcome.passed());
    }

    #[test]
    fn an_unknown_field_is_unevaluable() {
        for expression in [
            "urgent.confidence > 0.5", // a Noul has no confidence
            "urgent.choice == x",      // wrong primitive
            "team.score > 1",          // wrong primitive
            "team.probabilities.missing > 0",
            "severity.probabilities.99 > 0",
            "urgent.noul.extra > 1",
        ] {
            assert!(
                matches!(gate(expression), GateOutcome::Unevaluable { .. }),
                "{expression} was evaluable"
            );
        }
    }

    #[test]
    fn a_noul_has_no_confidence_and_the_gate_says_so() {
        // Guards against the community habit of synthesising a Noul confidence.
        match gate("urgent.confidence > 0.5") {
            GateOutcome::Unevaluable { reason } => {
                assert!(reason.contains("urgent.confidence"), "{reason}");
            }
            other => panic!("expected unevaluable, got {other:?}"),
        }
    }

    #[test]
    fn a_broken_side_of_an_and_is_reported_even_when_the_other_side_decides() {
        // Short-circuiting would hide the typo until the day the other side flips.
        assert!(matches!(
            gate("urgent.noul < 0.1 and typo.noul > 0.5"),
            GateOutcome::Unevaluable { .. }
        ));
        assert!(matches!(
            gate("urgent.noul > 0.9 or typo.noul > 0.5"),
            GateOutcome::Unevaluable { .. }
        ));
    }

    #[test]
    fn mismatched_types_are_unevaluable_with_an_explanation() {
        // Text compared with an ordering operator: the useful message names the two
        // operators that do work.
        match gate("team.choice > billing") {
            GateOutcome::Unevaluable { reason } => assert!(reason.contains("== or !="), "{reason}"),
            other => panic!("expected unevaluable, got {other:?}"),
        }
        // Text compared with a number, and the reverse.
        match gate("team.choice > 0.5") {
            GateOutcome::Unevaluable { reason } => {
                assert!(reason.contains("is text"), "{reason}");
            }
            other => panic!("expected unevaluable, got {other:?}"),
        }
        match gate("urgent.noul == billing") {
            GateOutcome::Unevaluable { reason } => assert!(reason.contains("number")),
            other => panic!("expected unevaluable, got {other:?}"),
        }
    }

    #[test]
    fn parse_errors_name_the_problem_and_the_place() {
        assert_eq!(parse(""), Err(ParseError::Empty));
        assert!(matches!(
            parse("urgent.noul = 0.5"),
            Err(ParseError::Expected { .. })
        ));
        assert!(matches!(
            parse("urgent.noul > 0.5 extra"),
            Err(ParseError::TrailingInput { .. })
        ));
        assert!(matches!(
            parse("(urgent.noul > 0.5"),
            Err(ParseError::UnexpectedEnd { .. })
        ));
        assert!(matches!(
            parse("urgent.noul >"),
            Err(ParseError::UnexpectedEnd { .. })
        ));
        assert!(matches!(
            parse("team.choice == 'unterminated"),
            Err(ParseError::UnterminatedString { .. })
        ));
        assert!(matches!(
            parse("urgent.noul > 0.5 && x.y > 1"),
            Err(ParseError::UnexpectedCharacter { .. })
        ));
        assert!(matches!(
            parse("urgent..noul > 1"),
            Err(ParseError::Expected { .. })
        ));
    }

    #[test]
    fn nothing_resembling_a_shell_is_accepted() {
        // There is no `eval` in this project, and the grammar has no way to express a
        // command. These are the shapes an injection attempt would take.
        for hostile in [
            "$(rm -rf /)",
            "`id`",
            "a; rm -rf /",
            "a | sh",
            "urgent.noul > 0.5; echo pwned",
            "__import__('os')",
            "{{7*7}}",
        ] {
            assert!(parse(hostile).is_err(), "accepted {hostile:?}");
        }
    }

    #[test]
    fn deep_nesting_is_refused_rather_than_overflowing_the_stack() {
        let deep = format!("{}urgent.noul > 0.5{}", "(".repeat(500), ")".repeat(500));
        assert_eq!(parse(&deep), Err(ParseError::TooDeep));
        let deep_not = format!("{}urgent.noul > 0.5", "not ".repeat(500));
        assert_eq!(parse(&deep_not), Err(ParseError::TooDeep));
    }

    #[test]
    fn an_over_long_expression_is_refused() {
        let long = format!("urgent.noul > 0.5 and {}", "x.y > 1 and ".repeat(1000));
        assert_eq!(parse(&long), Err(ParseError::TooLong));
    }

    #[test]
    fn nesting_just_inside_the_limit_still_parses() {
        let depth = MAX_DEPTH - 1;
        let expression = format!(
            "{}urgent.noul > 0.5{}",
            "(".repeat(depth),
            ")".repeat(depth)
        );
        assert!(parse(&expression).is_ok(), "rejected depth {depth}");
    }

    /// A comparison against a path that resolves in the fixture response.
    fn evaluable_expression() -> impl Strategy<Value = String> {
        let comparison = prop::sample::select(vec![
            "urgent.noul > 0.5",
            "urgent.noul < 0.5",
            "team.choice == billing",
            "team.choice != billing",
            "team.confidence >= 0.7",
            "severity.score < 2",
            "severity.probabilities.1 > 0.5",
            "team.probabilities.technical <= 0.3",
        ]);
        (
            comparison.clone(),
            prop::sample::select(vec!["and", "or"]),
            comparison,
        )
            .prop_map(|(a, joiner, b)| format!("{a} {joiner} {b}"))
            .boxed()
            .prop_union(
                prop::sample::select(vec![
                    "urgent.noul > 0.5",
                    "team.choice == billing",
                    "severity.score < 2",
                ])
                .prop_map(str::to_owned)
                .boxed(),
            )
    }

    /// Expressions that mix resolvable and unresolvable paths, plus a few malformed
    /// ones, so both the evaluator's success and failure branches are reached.
    fn expression() -> impl Strategy<Value = String> {
        let term = prop::sample::select(vec![
            "urgent.noul > 0.5",
            "team.choice == billing",
            "team.confidence >= 0.7",
            "severity.score < 2",
            "severity.probabilities.1 > 0.5",
            // Unresolvable: wrong primitive, unknown id, unknown option, Noul
            // confidence.
            "urgent.confidence > 0.5",
            "typo.noul > 0.5",
            "team.probabilities.ghost > 0.1",
            "team.score > 1",
            // Type mismatches.
            "team.choice > 0.5",
            "urgent.noul == billing",
        ]);
        let joiner = prop::sample::select(vec![" and ", " or "]);
        prop_oneof![
            term.clone().prop_map(str::to_owned),
            (term.clone(), joiner.clone(), term.clone()).prop_map(|(a, j, b)| format!("{a}{j}{b}")),
            (term.clone(), joiner, term).prop_map(|(a, j, b)| format!("not ({a}{j}{b})")),
        ]
    }

    proptest! {
        /// Totality: no input may panic the parser. This is the property the fuzz
        /// target in `fuzz/fuzz_targets/gate_expression.rs` extends to unstructured
        /// bytes.
        #[test]
        fn parsing_never_panics(input in ".{0,200}") {
            let _ = parse(&input);
        }

        /// Evaluation is total over input that actually parses.
        ///
        /// Random text never forms a valid expression, so a `.{0,200}` generator only
        /// ever exercises the parser's rejection branch and the evaluator is never
        /// called at all. This composes real expressions from real paths.
        #[test]
        fn evaluation_never_panics(input in expression()) {
            if let Ok(expression) = parse(&input) {
                let _ = evaluate(&expression, &response());
            }
        }

        /// A numeric comparison agrees with the same comparison done directly on the
        /// value. This is the property that would catch an inverted operator, which
        /// "does not panic" cannot see.
        #[test]
        fn a_numeric_comparison_agrees_with_plain_arithmetic(
            operator in prop::sample::select(vec![">", ">=", "<", "<=", "!="]),
            threshold in 0.0_f64..1.0,
        ) {
            // `urgent.noul` is 0.92 in the fixture response.
            let actual = 0.92_f64;
            let text = format!("urgent.noul {operator} {threshold}");
            let parsed = parse(&text).expect("a generated expression must parse");
            let expected = match operator {
                ">" => actual > threshold,
                ">=" => actual >= threshold,
                "<" => actual < threshold,
                "<=" => actual <= threshold,
                _ => (actual - threshold).abs() >= f64::EPSILON,
            };
            prop_assert_eq!(
                evaluate(&parsed, &response()) == GateOutcome::Passed,
                expected,
                "{} disagreed with plain arithmetic", text
            );
        }

        /// `not` really inverts, for every expression that evaluates at all.
        #[test]
        fn not_inverts_an_evaluable_gate(input in evaluable_expression()) {
            let plain = parse(&input).expect("a generated expression must parse");
            let negated = parse(&format!("not ({input})")).expect("must parse");
            let a = evaluate(&plain, &response());
            let b = evaluate(&negated, &response());
            if matches!(a, GateOutcome::Passed | GateOutcome::Failed) {
                prop_assert_ne!(
                    a == GateOutcome::Passed,
                    b == GateOutcome::Passed,
                    "`not` did not invert {}", input
                );
            }
        }

        /// A generated, always-valid expression must always parse, so the grammar the
        /// documentation describes is the grammar the parser accepts.
        #[test]
        fn well_formed_expressions_always_parse(
            field in prop::sample::select(vec!["urgent.noul", "severity.score", "team.confidence"]),
            operator in prop::sample::select(vec![">", ">=", "<", "<=", "==", "!="]),
            value in 0.0_f64..1.0,
        ) {
            let expression = format!("{field} {operator} {value}");
            prop_assert!(parse(&expression).is_ok(), "rejected {expression}");
        }

        /// An unevaluable gate never passes, over expressions that mix resolvable and
        /// unresolvable paths — so the generator actually reaches both outcomes rather
        /// than only producing unevaluable ones, which would make this a tautology.
        #[test]
        fn an_unevaluable_gate_never_passes(input in expression()) {
            if let Ok(expression) = parse(&input) {
                let outcome = evaluate(&expression, &response());
                if matches!(outcome, GateOutcome::Unevaluable { .. }) {
                    prop_assert!(!outcome.passed(), "an unevaluable gate reported a pass");
                }
            }
        }

        /// An expression containing an unresolvable path is *always* unevaluable, even
        /// when the other side of an `and`/`or` would decide the result on its own.
        /// This is what stops a typo hiding until the day the other side flips.
        #[test]
        fn one_broken_operand_makes_the_whole_gate_unevaluable(
            good in evaluable_expression(),
            operator in prop::sample::select(vec!["and", "or"]),
        ) {
            let text = format!("{good} {operator} nosuch.noul > 0.5");
            let parsed = parse(&text).expect("a generated expression must parse");
            prop_assert!(
                matches!(evaluate(&parsed, &response()), GateOutcome::Unevaluable { .. }),
                "{} hid a broken operand", text
            );
        }
    }
}
