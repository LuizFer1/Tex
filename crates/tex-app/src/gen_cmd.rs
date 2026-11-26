use tex_mask::{Builtin, GenKind, GenOptions, Generator, Random};

use crate::mask_cmd::{Command, MAX_SUGGESTIONS, Row, Section};

/// The most values one request makes.
pub const MAX_AMOUNT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub generator: usize,
    pub amount: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub suggestions: Vec<usize>,
    pub request: Option<Request>,
}

/// What a card was made from, so it is made again only when one of these changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub name: String,
    pub amount: usize,
    pub options: GenOptions,
}

/// What the panel shows for a `gerar`/`generate` command. The first word is matched against the
/// generator names for the suggestions; a request is made when it is exactly a name and the rest
/// is empty or an amount.
pub fn view(command: &Command, generators: &[Generator]) -> View {
    let (term, after) = split_term(&command.rest);
    let request = find(generators, term)
        .and_then(|generator| amount(after).map(|amount| Request { generator, amount }));
    View {
        suggestions: suggest(generators, term),
        request,
    }
}

fn split_term(rest: &str) -> (&str, &str) {
    let rest = rest.trim();
    rest.split_once(char::is_whitespace)
        .map_or((rest, ""), |(term, after)| (term, after.trim()))
}

fn find(generators: &[Generator], name: &str) -> Option<usize> {
    let name = tex_index::fold(name);
    generators
        .iter()
        .position(|generator| tex_index::fold(&generator.name) == name)
}

fn suggest(generators: &[Generator], term: &str) -> Vec<usize> {
    let term = tex_index::fold(term);
    generators
        .iter()
        .enumerate()
        .filter(|(_, generator)| tex_index::fold(&generator.name).contains(&term))
        .map(|(index, _)| index)
        .take(MAX_SUGGESTIONS)
        .collect()
}

/// Nothing is one value; digits are the amount, kept within 1 to `MAX_AMOUNT`.
fn amount(text: &str) -> Option<usize> {
    if text.is_empty() {
        return Some(1);
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(
        text.parse::<usize>()
            .unwrap_or(usize::MAX)
            .clamp(1, MAX_AMOUNT),
    )
}

/// The panel's lines: the line that generates again when a card is shown, then the matching
/// generators under their header.
pub fn rows(view: &View, has_card: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    if has_card {
        rows.push(Row::Again);
    }
    if !view.suggestions.is_empty() {
        rows.push(Row::Header(Section::Generators));
        rows.extend(view.suggestions.iter().map(|&index| Row::Generator(index)));
    }
    rows
}

/// The query after choosing the suggested generator `name`.
pub fn complete(command: &Command, name: &str) -> String {
    format!("{} {} ", command.keyword, name)
}

/// The key of the card `request` makes, `None` when the generator is gone.
pub fn key(request: Request, generators: &[Generator], options: GenOptions) -> Option<Key> {
    let generator = generators.get(request.generator)?;
    Some(Key {
        name: generator.name.clone(),
        amount: request.amount,
        options,
    })
}

/// The input the card shows: the generator's name, with the amount when it is more than one.
pub fn input(request: Request, generators: &[Generator]) -> String {
    let name = generators
        .get(request.generator)
        .map_or("", |generator| generator.name.as_str());
    if request.amount > 1 {
        format!("{name} × {}", request.amount)
    } else {
        name.to_owned()
    }
}

/// `amount` values of `generator`, or `None` when any of them fails.
pub fn produce(
    generator: &Generator,
    amount: usize,
    options: GenOptions,
    rng: &mut dyn Random,
) -> Option<Vec<String>> {
    (0..amount)
        .map(|_| generator.generate(rng, options))
        .collect()
}

/// The values of `request`. While the request is the same, the values kept in `cache` are given
/// back and nothing is drawn; otherwise they are made and kept. The cache is cleared when they
/// cannot be made, because the generator is gone or the random source failed.
pub fn values(
    cache: &mut Option<(Key, Vec<String>)>,
    request: Request,
    generators: &[Generator],
    options: GenOptions,
    rng: &mut dyn Random,
) -> Option<Vec<String>> {
    let Some(wanted) = key(request, generators, options) else {
        *cache = None;
        return None;
    };
    if let Some((_, values)) = cache.as_ref().filter(|(cached, _)| *cached == wanted) {
        return Some(values.clone());
    }
    let made = generators
        .get(request.generator)
        .and_then(|generator| produce(generator, request.amount, options, rng));
    *cache = made.as_ref().map(|values| (wanted, values.clone()));
    made
}

/// What Copy puts on the clipboard: one value per line, with Windows line breaks.
pub fn copy_text(values: &[String]) -> String {
    values.join("\r\n")
}

/// The shape of the values `generator` makes, with the password's characters and length. `None`
/// for a pattern, whose saved text is shown instead.
pub fn detail(generator: &Generator, options: GenOptions) -> Option<String> {
    match &generator.kind {
        GenKind::Builtin(Builtin::Cpf) => Some(if options.formatted {
            "###.###.###-##".to_owned()
        } else {
            "#".repeat(11)
        }),
        GenKind::Builtin(Builtin::Cnpj) => Some(if options.formatted {
            "##.###.###/####-##".to_owned()
        } else {
            "#".repeat(14)
        }),
        GenKind::Builtin(Builtin::CnpjAlpha) => Some(if options.formatted {
            "XX.XXX.XXX/XXXX-##".to_owned()
        } else {
            "X".repeat(12) + "##"
        }),
        GenKind::Builtin(Builtin::Password) => {
            let shape = if options.password_symbols {
                "Aa1!"
            } else {
                "Aa1"
            };
            Some(format!("{shape} · {}", options.password_length))
        }
        GenKind::Pattern(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mask_cmd::Kind;
    use tex_mask::{GenSource, GenSpec, compile_generators, default_generators};

    struct Seeded(u64);

    impl Random for Seeded {
        fn below(&mut self, n: u32) -> Option<u32> {
            let span = 1u64 << 32;
            let limit = span - span % u64::from(n);
            loop {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                let draw = self.0 >> 32;
                if draw < limit {
                    return Some((draw % u64::from(n)) as u32);
                }
            }
        }
    }

    struct Failing;

    impl Random for Failing {
        fn below(&mut self, _n: u32) -> Option<u32> {
            None
        }
    }

    fn generators() -> Vec<Generator> {
        let mut specs = default_generators(false);
        specs.push(GenSpec {
            name: "codigo".into(),
            source: GenSource::Pattern(r"[A-Z]{3}-\d{2}".into()),
        });
        compile_generators(&specs)
    }

    fn command(rest: &str) -> Command {
        Command {
            keyword: "gerar".into(),
            kind: Kind::Generate,
            rest: rest.into(),
        }
    }

    fn options() -> GenOptions {
        GenOptions {
            formatted: true,
            password_length: 16,
            password_symbols: true,
        }
    }

    fn request(generator: usize, amount: usize) -> Option<Request> {
        Some(Request { generator, amount })
    }

    #[test]
    fn a_name_alone_or_with_an_amount_is_a_request() {
        let generators = generators();
        assert_eq!(view(&command("cpf"), &generators).request, request(0, 1));
        assert_eq!(view(&command("CPF 5"), &generators).request, request(0, 5));
        assert_eq!(view(&command("cpf 5 "), &generators).request, request(0, 5));
        assert_eq!(
            view(&command("cpf   7"), &generators).request,
            request(0, 7)
        );
        assert_eq!(
            view(&command("codigo 3"), &generators).request,
            request(4, 3)
        );
        assert_eq!(view(&command("senha"), &generators).request, request(3, 1));
    }

    #[test]
    fn the_amount_is_clamped_and_must_be_digits() {
        let generators = generators();
        assert_eq!(view(&command("cpf 0"), &generators).request, request(0, 1));
        assert_eq!(
            view(&command("cpf 100"), &generators).request,
            request(0, MAX_AMOUNT)
        );
        assert_eq!(
            view(&command("cpf 101"), &generators).request,
            request(0, MAX_AMOUNT)
        );
        assert_eq!(
            view(&command("cpf 99999999999999999999999999999"), &generators).request,
            request(0, MAX_AMOUNT)
        );
        for rest in ["cpf x", "cpf 5x", "cpf -1", "cpf 1.5", "cpf 5 6", "cpf +2"] {
            assert_eq!(view(&command(rest), &generators).request, None, "{rest}");
        }
    }

    #[test]
    fn a_partial_name_only_suggests() {
        let generators = generators();
        let shown = view(&command("cn"), &generators);
        assert_eq!(shown.suggestions, vec![1, 2]);
        assert_eq!(shown.request, None);
        assert_eq!(view(&command("cp"), &generators).suggestions, vec![0]);
        assert_eq!(view(&command("cpf 5x"), &generators).suggestions, vec![0]);
        assert_eq!(
            view(&command("zzz"), &generators).suggestions,
            Vec::<usize>::new()
        );
    }

    #[test]
    fn an_empty_command_suggests_the_first_generators() {
        let generators = generators();
        let shown = view(&command(""), &generators);
        assert_eq!(shown.suggestions, vec![0, 1, 2, 3, 4]);
        assert_eq!(shown.request, None);
    }

    #[test]
    fn suggestions_are_capped() {
        let specs: Vec<GenSpec> = (0..9)
            .map(|index| GenSpec {
                name: format!("m{index}"),
                source: GenSource::Pattern(r"\d{3}".into()),
            })
            .collect();
        let generators = compile_generators(&specs);
        assert_eq!(
            view(&command("m"), &generators).suggestions.len(),
            MAX_SUGGESTIONS
        );
    }

    #[test]
    fn rows_put_again_first_then_the_matching_generators() {
        let generators = generators();
        let shown = view(&command("cn"), &generators);
        assert_eq!(
            rows(&shown, true),
            vec![
                Row::Again,
                Row::Header(Section::Generators),
                Row::Generator(1),
                Row::Generator(2),
            ]
        );
        assert_eq!(
            rows(&shown, false),
            vec![
                Row::Header(Section::Generators),
                Row::Generator(1),
                Row::Generator(2),
            ]
        );
        let none = view(&command("zzz"), &generators);
        assert_eq!(rows(&none, true), vec![Row::Again]);
        assert!(rows(&none, false).is_empty());
    }

    #[test]
    fn completing_writes_the_name_and_a_space() {
        assert_eq!(complete(&command("cn"), "cnpj"), "gerar cnpj ");
        let typed = Command {
            keyword: "GENERATE".into(),
            kind: Kind::Generate,
            rest: String::new(),
        };
        assert_eq!(complete(&typed, "cpf"), "GENERATE cpf ");
    }

    #[test]
    fn the_key_and_the_input_follow_the_request() {
        let generators = generators();
        let opts = options();
        assert_eq!(
            key(
                Request {
                    generator: 0,
                    amount: 1
                },
                &generators,
                opts
            ),
            Some(Key {
                name: "cpf".into(),
                amount: 1,
                options: opts,
            })
        );
        assert_eq!(
            key(
                Request {
                    generator: 9,
                    amount: 1
                },
                &generators,
                opts
            ),
            None
        );
        assert_eq!(
            input(
                Request {
                    generator: 0,
                    amount: 1
                },
                &generators
            ),
            "cpf"
        );
        assert_eq!(
            input(
                Request {
                    generator: 4,
                    amount: 5
                },
                &generators
            ),
            "codigo × 5"
        );
        assert_eq!(
            input(
                Request {
                    generator: 9,
                    amount: 1
                },
                &generators
            ),
            ""
        );
    }

    #[test]
    fn produce_makes_the_amount_or_nothing_when_the_source_fails() {
        let generators = generators();
        let mut rng = Seeded(0x9E37_79B9_7F4A_7C15);
        let values = produce(&generators[0], 5, options(), &mut rng).unwrap();
        assert_eq!(values.len(), 5);
        assert!(values.iter().all(|value| tex_mask::cpf_valid(value)));
        assert_eq!(
            produce(&generators[0], 0, options(), &mut rng),
            Some(Vec::new())
        );
        assert_eq!(produce(&generators[0], 3, options(), &mut Failing), None);
    }

    #[test]
    fn the_values_are_kept_until_the_request_changes() {
        let generators = generators();
        let opts = options();
        let mut cache = None;
        let request = Request {
            generator: 0,
            amount: 3,
        };
        let first = values(
            &mut cache,
            request,
            &generators,
            opts,
            &mut Seeded(0x9E37_79B9_7F4A_7C15),
        )
        .unwrap();
        assert_eq!(first.len(), 3);
        assert!(cache.is_some());
        assert_eq!(
            values(&mut cache, request, &generators, opts, &mut Failing),
            Some(first)
        );
        let other_amount = Request {
            generator: 0,
            amount: 4,
        };
        assert_eq!(
            values(&mut cache, other_amount, &generators, opts, &mut Failing),
            None
        );
        assert!(cache.is_none());
        let gone = Request {
            generator: 9,
            amount: 1,
        };
        assert_eq!(
            values(&mut cache, gone, &generators, opts, &mut Seeded(1)),
            None
        );
        assert!(cache.is_none());
    }

    #[test]
    fn a_changed_option_makes_the_values_again() {
        let generators = generators();
        let mut cache = None;
        let request = Request {
            generator: 3,
            amount: 1,
        };
        assert!(values(&mut cache, request, &generators, options(), &mut Seeded(7)).is_some());
        let longer = GenOptions {
            password_length: 20,
            ..options()
        };
        assert_eq!(
            values(&mut cache, request, &generators, longer, &mut Failing),
            None
        );
    }

    #[test]
    fn copy_joins_the_values_with_windows_line_breaks() {
        assert_eq!(copy_text(&["a".into(), "b".into()]), "a\r\nb");
        assert_eq!(copy_text(&[]), "");
    }

    #[test]
    fn detail_shows_the_shape_of_each_builtin() {
        let generators = generators();
        let formatted = options();
        assert_eq!(
            detail(&generators[0], formatted).as_deref(),
            Some("###.###.###-##")
        );
        assert_eq!(
            detail(&generators[1], formatted).as_deref(),
            Some("##.###.###/####-##")
        );
        assert_eq!(
            detail(&generators[2], formatted).as_deref(),
            Some("XX.XXX.XXX/XXXX-##")
        );
        assert_eq!(
            detail(&generators[3], formatted).as_deref(),
            Some("Aa1! · 16")
        );
        let plain = GenOptions {
            formatted: false,
            password_length: 12,
            password_symbols: false,
        };
        assert_eq!(detail(&generators[0], plain), Some("#".repeat(11)));
        assert_eq!(detail(&generators[1], plain), Some("#".repeat(14)));
        assert_eq!(detail(&generators[2], plain), Some("X".repeat(12) + "##"));
        assert_eq!(detail(&generators[3], plain).as_deref(), Some("Aa1 · 12"));
        assert_eq!(detail(&generators[4], formatted), None);
    }
}
