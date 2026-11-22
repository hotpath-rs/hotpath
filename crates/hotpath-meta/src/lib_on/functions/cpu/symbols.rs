//! Maps a binary's symbols to measured functions by their CPU patterns
//! (`identity::site_identity`): the full canonical path of the source
//! function, with `_` for every generic parameter.
//!
//! Both sides are tokenized and canonicalized, so `b::<impl a::Foo>::m`
//! (legacy mangling, impl outside the type's module) and `a::Foo::m` (v0)
//! meet. A `_` matches one type (`Worker<_>::run` matches `Worker<u8>::run`
//! and legacy `Worker<T>::run`). After the pattern a symbol may only continue
//! with the function's own generic arguments (`::<u8>`, v0) or a closure
//! (`::{closure#0}`, an async body's poll function); a nested named item is
//! its own function. The most specific pattern (fewest `_`) wins; a tie is
//! left unattributed.
//!
//! v0 mangling prints a trait default method's instance as `<Foo as Tr>::m`,
//! exactly like an override in `impl Tr for Foo`. The mangled form tells them
//! apart (`_RNvY` for the trait's item, `_RNvX` for an impl's), so a default
//! method's pattern `Tr::m` takes only the first and a trait impl's pattern
//! `<Foo as Tr>::m` only the second: an unmeasured override's samples never
//! go to the measured default method.

use std::collections::HashMap;

use crate::lib_on::functions::identity::{canonical, matching_gt, tokenize, Tok};

struct Pattern {
    toks: Vec<Tok>,
    wildcards: usize,
    display: &'static str,
    /// Two sites with one pattern (fns in two closures of one function):
    /// their symbols cannot be told apart.
    shared: bool,
}

pub(crate) struct SymbolMatcher {
    patterns: Vec<Pattern>,
    /// Identifier token -> patterns whose path contains it as its last
    /// identifier (the function's name).
    by_name: HashMap<String, Vec<usize>>,
}

impl SymbolMatcher {
    /// `patterns`: stats key -> CPU pattern.
    pub(crate) fn new<'a>(patterns: impl IntoIterator<Item = (&'static str, &'a str)>) -> Self {
        let mut list: Vec<Pattern> = Vec::new();
        let mut by_text: HashMap<String, usize> = HashMap::new();
        for (display, pattern) in patterns {
            if let Some(&index) = by_text.get(pattern) {
                list[index].shared = true;
                continue;
            }
            let toks = canonical(&tokenize(pattern));
            by_text.insert(pattern.to_string(), list.len());
            list.push(Pattern {
                wildcards: toks.iter().filter(|t| **t == Tok::Wild).count(),
                toks,
                display,
                shared: false,
            });
        }
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, pattern) in list.iter().enumerate() {
            if let Some(Tok::Id(name)) = pattern.toks.iter().rev().find(|t| matches!(t, Tok::Id(_)))
            {
                by_name.entry(name.clone()).or_default().push(index);
            }
        }
        Self {
            patterns: list,
            by_name,
        }
    }

    /// Measured function a symbol belongs to. `raw` is the mangled name as
    /// the object file has it, `demangled` its `{:#}` demangling.
    pub(crate) fn match_symbol(&self, raw: &str, demangled: &str) -> Option<&'static str> {
        let symbol = canonical(&tokenize(demangled));
        let kind = v0_kind(raw);
        let mut best: Option<&Pattern> = None;
        let mut tie = false;
        let mut seen: Vec<usize> = Vec::new();
        for tok in &symbol {
            let Tok::Id(id) = tok else { continue };
            let Some(candidates) = self.by_name.get(id) else {
                continue;
            };
            for &index in candidates {
                if seen.contains(&index) {
                    continue;
                }
                seen.push(index);
                let pattern = &self.patterns[index];
                if !matches_site(&pattern.toks, &symbol, kind) {
                    continue;
                }
                match best {
                    Some(b) if b.wildcards < pattern.wildcards => {}
                    Some(b) if b.wildcards == pattern.wildcards => tie = true,
                    _ => {
                        best = Some(pattern);
                        tie = false;
                    }
                }
            }
        }
        best.filter(|p| !tie && !p.shared).map(|p| p.display)
    }
}

/// Kind of a v0 symbol's innermost function path: `X` (trait impl item),
/// `Y` (trait item, i.e. a default method instance), `M` (inherent impl
/// item); `None` for legacy symbols and other paths.
fn v0_kind(raw: &str) -> Option<char> {
    // `_R` on Linux, `__R` on macOS.
    let mut s = raw.strip_prefix("__R").or_else(|| raw.strip_prefix("_R"))?;
    s = s.trim_start_matches(|c: char| c.is_ascii_digit());
    loop {
        if let Some(rest) = s.strip_prefix('I') {
            s = rest;
        } else if let Some(rest) = s.strip_prefix("NC") {
            s = rest;
        } else {
            break;
        }
    }
    s.strip_prefix("Nv")?.chars().next()
}

/// Length of the type starting at `toks[0]`: up to a top-level `,`, `;`,
/// `as` or closing bracket.
fn type_len(toks: &[Tok]) -> usize {
    let mut depth = 0usize;
    for (k, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Lt | Tok::P('(') | Tok::P('[') => depth += 1,
            Tok::Gt | Tok::P(')') | Tok::P(']') => {
                if depth == 0 {
                    return k;
                }
                depth -= 1;
            }
            Tok::Comma | Tok::P(';') | Tok::As if depth == 0 => return k,
            _ => {}
        }
    }
    toks.len()
}

/// Symbol tokens consumed when `pattern` matches a prefix of `symbol`.
fn match_prefix(pattern: &[Tok], symbol: &[Tok]) -> Option<usize> {
    let Some((first, rest)) = pattern.split_first() else {
        return Some(0);
    };
    if *first == Tok::Wild {
        // A path type spans `a::B<C>`; try the longest type first.
        let longest = type_len(symbol);
        return (1..=longest)
            .rev()
            .find_map(|len| match_prefix(rest, &symbol[len..]).map(|used| len + used));
    }
    if symbol.first() == Some(first) {
        return match_prefix(rest, &symbol[1..]).map(|used| used + 1);
    }
    None
}

fn matches_site(pattern: &[Tok], symbol: &[Tok], kind: Option<char>) -> bool {
    let trait_pattern = pattern.first() == Some(&Tok::Lt) && pattern.contains(&Tok::As);
    let rewritten;
    let symbol = if kind == Some('Y') {
        if trait_pattern {
            return false;
        }
        // `<Foo as Tr>::m` instance of the default method `Tr::m`.
        match default_method_path(symbol) {
            Some(path) => {
                rewritten = path;
                &rewritten
            }
            None => return false,
        }
    } else {
        symbol
    };
    let Some(used) = match_prefix(pattern, symbol) else {
        return false;
    };
    let rest = &symbol[used..];
    rest.is_empty()
        || (rest[0] == Tok::Sep && matches!(rest.get(1), Some(Tok::Lt) | Some(Tok::Closure)))
}

/// `<X as Tr>::rest` -> `Tr::rest`.
fn default_method_path(symbol: &[Tok]) -> Option<Vec<Tok>> {
    if symbol.first() != Some(&Tok::Lt) {
        return None;
    }
    let end = matching_gt(symbol, 0)?;
    let mut depth = 0usize;
    let as_index = (1..end).find(|&k| {
        match symbol[k] {
            Tok::Lt => depth += 1,
            Tok::Gt => depth = depth.saturating_sub(1),
            _ => {}
        }
        depth == 0 && symbol[k] == Tok::As
    })?;
    let mut path = symbol[as_index + 1..end].to_vec();
    path.extend_from_slice(&symbol[end + 1..]);
    Some(path)
}

#[cfg(test)]
mod tests {
    use crate::lib_on::functions::cpu::symbols::SymbolMatcher;

    /// Sites and symbols from one experiment binary, built with rustc 1.100
    /// (v0 mangling) and 1.89 (legacy).
    const SITES: &[(&str, &str)] = &[
        ("by_ref", "names::a::Foo::by_ref"),
        ("generic_method", "names::a::Foo::generic_method"),
        ("async_m", "names::a::Foo::async_m"),
        ("run", "names::a::Worker<_>::run"),
        ("same_u8", "names::a::Multi<u8>::same"),
        ("same_u16", "names::a::Multi<u16>::same"),
        ("render", "<names::a::Foo as names::b::Render>::render"),
        ("display", "<names::a::Foo as core::fmt::Display>::fmt"),
        ("debug", "<names::a::Foo as core::fmt::Debug>::fmt"),
        ("hello", "names::b::Defaulted::hello"),
        ("blanket", "<_ as names::b::Blanket>::blanket"),
        ("on_ref", "<&names::a::Foo as names::b::OnRef>::on_ref"),
        (
            "wrapper",
            "<names::a::Wrapper<_> as names::b::OnRef>::on_ref",
        ),
        (
            "canvas",
            "<&names::a::Canvas<_> as names::b::OnRef>::on_ref",
        ),
        ("gt_u8", "<names::a::Foo as names::b::GenTrait<u8>>::gt"),
        ("tuple", "<(u8, u8) as names::b::Blanket2>::tuple"),
        ("dyn", "<dyn names::b::DynT>::inherent_on_dyn"),
        ("free", "names::b::free_fn"),
        ("cg", "app::Arr<N>::cg"),
    ];

    fn matcher() -> SymbolMatcher {
        SymbolMatcher::new(SITES.iter().copied())
    }

    #[test]
    fn v0_symbols() {
        let m = matcher();
        let cases: &[(&str, &str, Option<&str>)] = &[
            (
                "_RNvMNtCs1_5names1aNtB2_3Foo6by_ref",
                "names::a::Foo::by_ref",
                Some("by_ref"),
            ),
            (
                "_RINvMNtCs1_5names1aNtB3_3Foo14generic_methodhE",
                "names::a::Foo::generic_method::<u8>",
                Some("generic_method"),
            ),
            (
                "_RNCNvMNtCs1_5names1aNtB4_3Foo7async_m0B6_",
                "names::a::Foo::async_m::{closure#0}",
                Some("async_m"),
            ),
            (
                "_RNvMNtCs1_5names1aINtB2_6WorkerhE3run",
                "names::a::Worker<u8>::run",
                Some("run"),
            ),
            (
                "_RNvMNtCs1_5names1aINtB2_6WorkerReE3run",
                "names::a::Worker<&str>::run",
                Some("run"),
            ),
            (
                "_RNvM0_NtCs1_5names1aINtB3_5MultihE4same",
                "names::a::Multi<u8>::same",
                Some("same_u8"),
            ),
            (
                "_RNvM1_NtCs1_5names1aINtB3_5MultitE4same",
                "names::a::Multi<u16>::same",
                Some("same_u16"),
            ),
            (
                "_RNvXNtCs1_5names1bNtNtB4_1a3FooNtB2_6Render6render",
                "<names::a::Foo as names::b::Render>::render",
                Some("render"),
            ),
            (
                "_RNvXs_NtCs1_5names1bNtNtB5_1a3FooNtNtCs2_4core3fmt7Display3fmt",
                "<names::a::Foo as core::fmt::Display>::fmt",
                Some("display"),
            ),
            (
                "_RNvXs0_NtCs1_5names1bNtNtB5_1a3FooNtNtCs2_4core3fmt5Debug3fmt",
                "<names::a::Foo as core::fmt::Debug>::fmt",
                Some("debug"),
            ),
            // default method instances (`Y`) -> the default method
            (
                "_RNvYNtNtCs1_5names1a3FooNtNtB4_1b9Defaulted5hello",
                "<names::a::Foo as names::b::Defaulted>::hello",
                Some("hello"),
            ),
            (
                "_RNvYmNtNtCs1_5names1b9Defaulted5hello",
                "<u32 as names::b::Defaulted>::hello",
                Some("hello"),
            ),
            // an unmeasured override (`X`) -> nothing
            (
                "_RNvXs7_NtCs1_5names1bNtB5_3OvrNtB5_9Defaulted5hello",
                "<names::b::Ovr as names::b::Defaulted>::hello",
                None,
            ),
            (
                "_RNvXs1_NtCs1_5names1bhNtB5_7Blanket7blanket",
                "<u8 as names::b::Blanket>::blanket",
                Some("blanket"),
            ),
            (
                "_RNvXs2_NtCs1_5names1bReNtB5_7Blanket7blanket",
                "<&str as names::b::Blanket>::blanket",
                Some("blanket"),
            ),
            (
                "_RNvXs3_NtCs1_5names1bRNtNtB5_1a3FooNtB5_5OnRef6on_ref",
                "<&names::a::Foo as names::b::OnRef>::on_ref",
                Some("on_ref"),
            ),
            (
                "_RNvXs4_NtCs1_5names1bINtNtB5_1a7WrapperhENtB5_5OnRef6on_ref",
                "<names::a::Wrapper<u8> as names::b::OnRef>::on_ref",
                Some("wrapper"),
            ),
            (
                "_RNvXs5_NtCs1_5names1bRINtNtB5_1a6CanvasNCNvB7_4main0ENtB5_5OnRef6on_ref",
                "<&names::a::Canvas<names::main::{closure#0}> as names::b::OnRef>::on_ref",
                Some("canvas"),
            ),
            (
                "_RNvXs6_NtCs1_5names1bNtNtB5_1a3FooINtB5_8GenTraithE2gt",
                "<names::a::Foo as names::b::GenTrait<u8>>::gt",
                Some("gt_u8"),
            ),
            (
                "_RNvXs8_NtCs1_5names1bNtNtB5_1a3FooINtB5_8GenTraittE2gt",
                "<names::a::Foo as names::b::GenTrait<u16>>::gt",
                None,
            ),
            (
                "_RNvXs9_NtCs1_5names1bThhENtB5_8Blanket25tuple",
                "<(u8, u8) as names::b::Blanket2>::tuple",
                Some("tuple"),
            ),
            (
                "_RNvMsa_NtCs1_5names1bDNtB5_4DynTEL_15inherent_on_dyn",
                "<dyn names::b::DynT>::inherent_on_dyn",
                Some("dyn"),
            ),
            (
                "_RNvNtCs1_5names1b7free_fn",
                "names::b::free_fn",
                Some("free"),
            ),
            // a named item nested in a measured fn is its own function
            (
                "_RNvNvNtCs1_5names1b7free_fn6helper",
                "names::b::free_fn::helper",
                None,
            ),
            // const generics print the parameter name: unattributed, never wrong
            ("_RNvMCs1_3appINtB2_3ArrKj4_E2cg", "app::Arr<4>::cg", None),
        ];
        for &(raw, demangled, expected) in cases {
            assert_eq!(m.match_symbol(raw, demangled), expected, "{demangled}");
        }
    }

    #[test]
    fn legacy_symbols() {
        let m = matcher();
        let cases: &[(&str, Option<&str>)] = &[
            ("names::b::<impl names::a::Foo>::by_ref", Some("by_ref")),
            (
                "names::b::<impl names::a::Foo>::generic_method",
                Some("generic_method"),
            ),
            (
                "names::b::<impl names::a::Foo>::async_m::{{closure}}",
                Some("async_m"),
            ),
            ("names::b::<impl names::a::Worker<T>>::run", Some("run")),
            (
                "names::b::<impl names::a::Multi<u16>>::same",
                Some("same_u16"),
            ),
            (
                "<names::a::Foo as names::b::Render>::render",
                Some("render"),
            ),
            (
                "names::b::<impl core::fmt::Debug for names::a::Foo>::fmt",
                Some("debug"),
            ),
            ("names::b::Defaulted::hello", Some("hello")),
            ("<names::b::Ovr as names::b::Defaulted>::hello", None),
            ("<T as names::b::Blanket>::blanket", Some("blanket")),
            (
                "<names::a::Wrapper<T> as names::b::OnRef>::on_ref",
                Some("wrapper"),
            ),
            (
                "<&names::a::Canvas<F> as names::b::OnRef>::on_ref",
                Some("canvas"),
            ),
            ("<(u8,u8) as names::b::Blanket2>::tuple", Some("tuple")),
            ("names::b::free_fn", Some("free")),
        ];
        for &(demangled, expected) in cases {
            let raw = "_ZN5names1b7free_fn17h0123456789abcdefE";
            assert_eq!(m.match_symbol(raw, demangled), expected, "{demangled}");
        }
    }

    #[test]
    fn macos_symbols_carry_an_extra_underscore() {
        let m = matcher();
        let demangled = "<names::a::Foo as names::b::Defaulted>::hello";
        let raw = "__RNvYNtNtCs1_5names1a3FooNtNtB4_1b9Defaulted5hello";
        assert_eq!(m.match_symbol(raw, demangled), Some("hello"));
        let raw = "__RNvXs7_NtCs1_5names1bNtB5_3OvrNtB5_9Defaulted5hello";
        assert_eq!(
            m.match_symbol(raw, "<names::b::Ovr as names::b::Defaulted>::hello"),
            None
        );
    }

    #[test]
    fn labeled_function_maps_to_its_label() {
        let m = SymbolMatcher::new([("custom_label", "mevlog::compute_hash")]);
        let raw = "_ZN6mevlog12compute_hash28_$u7b$$u7b$closure$u7d$$u7d$17h0123456789abcdefE";
        assert_eq!(
            m.match_symbol(raw, "mevlog::compute_hash::{{closure}}"),
            Some("custom_label")
        );
    }

    #[test]
    fn shared_patterns_and_ties_stay_unattributed() {
        let m = SymbolMatcher::new([
            ("first", "app::f::{{closure}}::nested"),
            ("second", "app::f::{{closure}}::nested"),
            ("x", "app::Pair<_, u8>::get"),
            ("y", "app::Pair<u8, _>::get"),
        ]);
        let legacy = "_ZN3app1f28_$u7b$$u7b$closure$u7d$$u7d$6nested17h0123456789abcdefE";
        assert_eq!(m.match_symbol(legacy, "app::f::{{closure}}::nested"), None);
        assert_eq!(
            m.match_symbol("_RNvMCs1_3app4get", "app::Pair<u8, u8>::get"),
            None
        );
        assert_eq!(
            m.match_symbol("_RNvMCs1_3app4get", "app::Pair<u16, u8>::get"),
            Some("x")
        );
    }
}
