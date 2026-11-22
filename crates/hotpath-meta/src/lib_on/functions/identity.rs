//! Identity of a `#[measure]` function: its stats key and CPU symbol pattern.
//!
//! The macro emits an empty `fn __hotpath_site() {}` into the measured body
//! and passes `type_name_of_val(&__hotpath_site)`. A nested item cannot use
//! the outer generics, so it has one instantiation and one type name for every
//! instantiation of the measured function: `a::Foo::m`, `b::<impl a::Foo>::m`,
//! `<a::Foo as b::Tr>::m`, `b::<impl core::fmt::Display for a::Foo>::fmt`,
//! `<_ as b::Blanket>::m` (generics print as `_`), `b::Tr::default_m`.
//!
//! The stats key keeps today's `module::Type::fn` shape where it can and puts
//! a trait impl's self type and trait in a short qualified segment
//! (`module::<&Block as Widget>::render`). The CPU pattern is the full
//! canonical path, so it can be matched against demangled symbols.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::lib_on::Location;

/// Name of the nested item the macro emits; its type name ends in it.
const SITE_ITEM: &str = "__hotpath_site";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Tok {
    Id(String),
    Sep,
    Lt,
    Gt,
    Comma,
    As,
    For,
    Impl,
    Wild,
    Closure,
    Arrow,
    Kw(&'static str),
    P(char),
}

/// Tokens of a type name or demangled symbol, with lifetimes and the comma
/// after them dropped (`type_name` prints `'_`, v0 symbols omit them).
pub(crate) fn tokenize(s: &str) -> Vec<Tok> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == ':' && chars.get(i + 1) == Some(&':') {
            out.push(Tok::Sep);
            i += 2;
        } else if c == '-' && chars.get(i + 1) == Some(&'>') {
            out.push(Tok::Arrow);
            i += 2;
        } else if c == '{' {
            // `{{closure}}` (type names, legacy symbols), `{closure#0}` (v0),
            // other compiler-generated segments such as `{shim:vtable#0}`.
            let start = i;
            let mut depth = 0;
            while i < chars.len() {
                match chars[i] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            out.push(if text.contains("closure") {
                Tok::Closure
            } else {
                Tok::Id(text)
            });
        } else if c == '\'' {
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if chars.get(i) == Some(&',') {
                i += 1;
            }
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            out.push(match word.as_str() {
                "_" => Tok::Wild,
                "as" => Tok::As,
                "for" => Tok::For,
                "impl" => Tok::Impl,
                "dyn" => Tok::Kw("dyn"),
                "mut" => Tok::Kw("mut"),
                "const" => Tok::Kw("const"),
                "fn" => Tok::Kw("fn"),
                "unsafe" => Tok::Kw("unsafe"),
                "extern" => Tok::Kw("extern"),
                _ => Tok::Id(word),
            });
        } else {
            out.push(match c {
                '<' => Tok::Lt,
                '>' => Tok::Gt,
                ',' => Tok::Comma,
                _ => Tok::P(c),
            });
            i += 1;
        }
    }
    // Lists left empty or with a trailing comma by the dropped lifetimes.
    let mut toks: Vec<Tok> = Vec::with_capacity(out.len());
    for tok in out {
        if tok == Tok::Gt && toks.last() == Some(&Tok::Comma) {
            toks.pop();
        }
        if tok == Tok::Gt && toks.last() == Some(&Tok::Lt) {
            toks.pop();
            continue;
        }
        toks.push(tok);
    }
    toks
}

pub(crate) fn matching_gt(toks: &[Tok], lt: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (k, tok) in toks.iter().enumerate().skip(lt) {
        match tok {
            Tok::Lt => depth += 1,
            Tok::Gt => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

fn balanced(toks: &[Tok]) -> bool {
    let mut depth = 0i32;
    for tok in toks {
        match tok {
            Tok::Lt => depth += 1,
            Tok::Gt => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    depth == 0
}

/// Index of `target` at bracket depth 0 within `toks`.
fn find_top_level(toks: &[Tok], target: &Tok) -> Option<usize> {
    let mut depth = 0i32;
    for (k, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Lt | Tok::P('(') | Tok::P('[') => depth += 1,
            Tok::Gt | Tok::P(')') | Tok::P(']') => depth -= 1,
            _ if depth == 0 && tok == target => return Some(k),
            _ => {}
        }
    }
    None
}

/// The path's impl segment: `mod::<impl X>` / `mod::<impl Tr for X>` (impl in
/// neither the type's nor the trait's module) or a leading `<X as Tr>` / `<X>`.
struct ImplSegment<'a> {
    /// Tokens before the segment, without the separator (`mod`); empty for a
    /// leading segment.
    prefix: &'a [Tok],
    self_ty: &'a [Tok],
    trait_: Option<&'a [Tok]>,
    /// Tokens after the closing `>`, starting with `::`.
    rest: &'a [Tok],
}

fn impl_segment(toks: &[Tok]) -> Option<ImplSegment<'_>> {
    let start = (0..toks.len()).find(|&i| {
        toks[i] == Tok::Lt
            && ((i > 0 && toks[i - 1] == Tok::Sep && toks.get(i + 1) == Some(&Tok::Impl)) || i == 0)
    })?;
    let end = matching_gt(toks, start)?;
    let rest = &toks[end + 1..];
    if rest.first() != Some(&Tok::Sep) {
        return None;
    }
    let prefix = if start == 0 {
        &toks[..0]
    } else {
        &toks[..start - 1]
    };
    let inner = &toks[start + 1..end];
    let (self_ty, trait_) = if let Some(inner) = inner.strip_prefix(&[Tok::Impl]) {
        match find_top_level(inner, &Tok::For) {
            Some(k) => (&inner[k + 1..], Some(&inner[..k])),
            None => (inner, None),
        }
    } else {
        match find_top_level(inner, &Tok::As) {
            Some(k) => (&inner[..k], Some(&inner[k + 1..])),
            None => (inner, None),
        }
    };
    if self_ty.is_empty() || trait_.is_some_and(<[Tok]>::is_empty) {
        return None;
    }
    Some(ImplSegment {
        prefix,
        self_ty,
        trait_,
        rest,
    })
}

fn is_plain_path(toks: &[Tok]) -> bool {
    matches!(toks.first(), Some(Tok::Id(_)))
}

/// Full path with the impl segment in its canonical form: `X::m` for an
/// inherent impl on a plain path, `<X>::m` otherwise, `<X as Tr>::m` for a
/// trait impl. Both type names and demangled symbols go through it, so the
/// impl's module placement does not matter.
pub(crate) fn canonical(toks: &[Tok]) -> Vec<Tok> {
    let Some(seg) = impl_segment(toks) else {
        return toks.to_vec();
    };
    let mut out = Vec::with_capacity(toks.len());
    match seg.trait_ {
        Some(trait_) => {
            out.push(Tok::Lt);
            out.extend_from_slice(seg.self_ty);
            out.push(Tok::As);
            out.extend_from_slice(trait_);
            out.push(Tok::Gt);
        }
        None if is_plain_path(seg.self_ty) => out.extend_from_slice(seg.self_ty),
        None => {
            out.push(Tok::Lt);
            out.extend_from_slice(seg.self_ty);
            out.push(Tok::Gt);
        }
    }
    out.extend_from_slice(seg.rest);
    out
}

/// Removes generic lists that hold only wildcards (`Worker<_>` -> `Worker`):
/// every instantiation of a generic impl reports under one key. Lists naming
/// a concrete type stay (`Multi<u8>`), they tell impls apart.
fn drop_wildcard_lists(toks: &[Tok]) -> Vec<Tok> {
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if toks[i] == Tok::Lt && matches!(out.last(), Some(Tok::Id(_))) {
            if let Some(end) = matching_gt(toks, i) {
                if toks[i + 1..end]
                    .iter()
                    .all(|t| matches!(t, Tok::Wild | Tok::Comma))
                {
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

/// Every path in `toks` cut to its last segment (`core::fmt::Debug` -> `Debug`).
fn last_segments(toks: &[Tok]) -> Vec<Tok> {
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if matches!(toks[i], Tok::Id(_))
            && toks.get(i + 1) == Some(&Tok::Sep)
            && matches!(toks.get(i + 2), Some(Tok::Id(_)))
        {
            i += 2;
            continue;
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

pub(crate) fn render(toks: &[Tok]) -> String {
    let mut s = String::new();
    for (k, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Id(word) => {
                if matches!(toks.get(k.wrapping_sub(1)), Some(Tok::Id(_))) {
                    s.push(' ');
                }
                s.push_str(word);
            }
            Tok::Kw(word) => {
                s.push_str(word);
                if toks.get(k + 1) != Some(&Tok::P('(')) {
                    s.push(' ');
                }
            }
            Tok::Sep => s.push_str("::"),
            Tok::Lt => s.push('<'),
            Tok::Gt => s.push('>'),
            Tok::Comma => s.push_str(", "),
            Tok::As => s.push_str(" as "),
            Tok::For => s.push_str(" for "),
            Tok::Impl => s.push_str("impl "),
            Tok::Wild => s.push('_'),
            Tok::Closure => s.push_str("{{closure}}"),
            Tok::Arrow => s.push_str(" -> "),
            Tok::P(';') => s.push_str("; "),
            Tok::P('+') => s.push_str(" + "),
            Tok::P(c) => s.push(*c),
        }
    }
    s
}

/// The site's path from the nested item's type name: the item segment and,
/// for an `async fn` (whose body is a coroutine), one `{{closure}}` removed.
fn site_path(site_type_name: &str, is_async: bool) -> &str {
    let path = site_type_name
        .strip_suffix(SITE_ITEM)
        .and_then(|p| p.strip_suffix("::"))
        .unwrap_or(site_type_name);
    if is_async {
        path.strip_suffix("::{{closure}}").unwrap_or(path)
    } else {
        path
    }
}

/// Short stats key: the impl segment's paths cut to their last segment and
/// placed under the impl's module (`module::Type::fn`,
/// `module::<&Block as Widget>::render`); other paths unchanged.
fn short_key(toks: &[Tok], module_path: &str) -> String {
    let Some(seg) = impl_segment(toks) else {
        return render(&drop_wildcard_lists(toks));
    };
    let prefix = if seg.prefix.is_empty() {
        module_path.to_string()
    } else {
        render(seg.prefix)
    };
    let self_ty = render(&last_segments(&drop_wildcard_lists(seg.self_ty)));
    let segment = match seg.trait_ {
        Some(trait_) => {
            let trait_ = render(&last_segments(&drop_wildcard_lists(trait_)));
            format!("<{self_ty} as {trait_}>")
        }
        None if is_plain_path(seg.self_ty) => self_ty,
        None => format!("<{self_ty}>"),
    };
    format!(
        "{prefix}::{segment}{}",
        render(&drop_wildcard_lists(seg.rest))
    )
}

/// Stats key and CPU pattern of one site.
pub(crate) struct SiteIdentity {
    pub(crate) key: String,
    /// Unique per source method: the full canonical path, with wildcards.
    pub(crate) pattern: String,
    /// The full path without wildcard lists, the key's collision fallback.
    full: String,
}

pub(crate) fn site_identity(
    module_path: &str,
    site_type_name: &str,
    is_async: bool,
) -> SiteIdentity {
    let path = site_path(site_type_name, is_async);
    let toks = tokenize(path);
    if !balanced(&toks) {
        // Unbalanced: an output shape the tokenizer does not know. The raw path
        // is still unique per site, unlike `module::fn`.
        return SiteIdentity {
            key: path.to_string(),
            pattern: path.to_string(),
            full: path.to_string(),
        };
    }
    let canonical = canonical(&toks);
    SiteIdentity {
        key: short_key(&toks, module_path),
        pattern: render(&canonical),
        full: render(&drop_wildcard_lists(&canonical)),
    }
}

struct Site {
    pattern: &'static str,
    location: &'static Location,
}

type SitesMap = std::sync::RwLock<HashMap<&'static str, Site>>;

/// Stats key -> the site that claimed it, to keep two sites from sharing one.
static SITES: OnceLock<SitesMap> = OnceLock::new();

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Stats key of a `#[measure]` function without a `label`, called once per
/// site. A key another site already claimed (two types named alike in one
/// module, or fns in two closures of one function) is never shared: this site
/// falls back to its full path, then to the path plus its line and column,
/// and the collision is reported.
#[doc(hidden)]
pub fn function_name(
    module_path: &'static str,
    site_type_name: &'static str,
    is_async: bool,
    location: &'static Location,
) -> &'static str {
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let identity = site_identity(module_path, site_type_name, is_async);
    let pattern = leak(identity.pattern);
    let sites = SITES.get_or_init(|| std::sync::RwLock::new(HashMap::new()));
    let Ok(mut sites) = sites.write() else {
        return leak(identity.key);
    };
    let free = |sites: &HashMap<&'static str, Site>, key: &str| match sites.get(key) {
        None => true,
        Some(site) => site.pattern == pattern && std::ptr::eq(site.location, location),
    };
    let key = [
        identity.key.clone(),
        identity.full,
        format!("{}@{}:{}", identity.key, location.line, location.column),
    ]
    .into_iter()
    .find(|key| free(&sites, key))
    .unwrap_or_else(|| {
        format!(
            "{}@{}:{}:{}",
            identity.key, location.file, location.line, location.column
        )
    });
    if key != identity.key {
        if let Some(taken) = sites.get(identity.key.as_str()) {
            eprintln!(
                "[hotpath-meta] function name {} is used by {}:{} and {}:{}; reporting the second as {key}",
                identity.key,
                taken.location.file,
                taken.location.line,
                location.file,
                location.line,
            );
        }
    }
    if let Some((&key, _)) = sites.get_key_value(key.as_str()) {
        return key;
    }
    let key = leak(key);
    sites.insert(key, Site { pattern, location });
    key
}

/// CPU symbol pattern of a `label`ed function, which keeps the label as its key.
#[doc(hidden)]
pub fn label_pattern(site_type_name: &'static str, is_async: bool) -> &'static str {
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    leak(site_identity("", site_type_name, is_async).pattern)
}

/// Stats key -> CPU symbol pattern of every unlabeled site called so far.
#[cfg(feature = "hotpath-cpu-meta")]
pub(crate) fn site_patterns() -> HashMap<&'static str, &'static str> {
    SITES
        .get()
        .and_then(|m| {
            m.read()
                .ok()
                .map(|g| g.iter().map(|(&key, site)| (key, site.pattern)).collect())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use crate::lib_on::functions::identity::site_identity;

    // `type_name_of_val` of the nested item, as rustc prints it (verified on
    // 1.89, 1.100 and nightly), -> (stats key, CPU pattern).
    #[test]
    fn identities_of_rustc_type_names() {
        let cases: &[(&str, bool, &str, &str)] = &[
            // free fn and a fn nested in another one
            (
                "app::m::free_fn::__hotpath_site",
                false,
                "app::m::free_fn",
                "app::m::free_fn",
            ),
            (
                "app::m::outer::helper::__hotpath_site",
                false,
                "app::m::outer::helper",
                "app::m::outer::helper",
            ),
            // inherent impl in the type's module, any receiver or none
            (
                "app::a::Foo::by_ref::__hotpath_site",
                false,
                "app::a::Foo::by_ref",
                "app::a::Foo::by_ref",
            ),
            // inherent impl in another module: the impl's module, like the type
            // segment `module_path!()` gives
            (
                "app::b::<impl app::a::Foo>::by_ref::__hotpath_site",
                false,
                "app::b::Foo::by_ref",
                "app::a::Foo::by_ref",
            ),
            // generic impl: one key for every instantiation
            (
                "app::b::<impl app::a::Worker<_>>::run::__hotpath_site",
                false,
                "app::b::Worker::run",
                "app::a::Worker<_>::run",
            ),
            (
                "app::a::G<_>::g::__hotpath_site",
                false,
                "app::a::G::g",
                "app::a::G<_>::g",
            ),
            // concrete impls of one generic type stay apart
            (
                "app::b::<impl app::a::Multi<u8>>::same::__hotpath_site",
                false,
                "app::b::Multi<u8>::same",
                "app::a::Multi<u8>::same",
            ),
            // trait impls: the trait tells `Display::fmt` from `Debug::fmt`
            (
                "app::b::<impl core::fmt::Display for app::a::Foo>::fmt::__hotpath_site",
                false,
                "app::b::<Foo as Display>::fmt",
                "<app::a::Foo as core::fmt::Display>::fmt",
            ),
            (
                "app::b::<impl core::fmt::Debug for app::a::Foo>::fmt::__hotpath_site",
                false,
                "app::b::<Foo as Debug>::fmt",
                "<app::a::Foo as core::fmt::Debug>::fmt",
            ),
            (
                "<app::a::Foo as app::b::GenTrait<u8>>::gt::__hotpath_site",
                false,
                "app::b::<Foo as GenTrait<u8>>::gt",
                "<app::a::Foo as app::b::GenTrait<u8>>::gt",
            ),
            (
                "<&app::a::Canvas<'_, _> as app::b::OnRef>::on_ref::__hotpath_site",
                false,
                "app::b::<&Canvas as OnRef>::on_ref",
                "<&app::a::Canvas<_> as app::b::OnRef>::on_ref",
            ),
            // trait default method: one key across implementors
            (
                "app::b::Defaulted::hello::__hotpath_site",
                false,
                "app::b::Defaulted::hello",
                "app::b::Defaulted::hello",
            ),
            // blanket impl
            (
                "<_ as app::b::Blanket>::blanket::__hotpath_site",
                false,
                "app::b::<_ as Blanket>::blanket",
                "<_ as app::b::Blanket>::blanket",
            ),
            // non-path self types
            (
                "<(u8, u8) as app::b::Pair>::pair::__hotpath_site",
                false,
                "app::b::<(u8, u8) as Pair>::pair",
                "<(u8, u8) as app::b::Pair>::pair",
            ),
            (
                "<[u8; 3] as app::OnArr>::on_arr::__hotpath_site",
                false,
                "app::b::<[u8; 3] as OnArr>::on_arr",
                "<[u8; 3] as app::OnArr>::on_arr",
            ),
            (
                "<dyn core::fmt::Debug + core::marker::Send as app::OnArr>::on_arr::__hotpath_site",
                false,
                "app::b::<dyn Debug + Send as OnArr>::on_arr",
                "<dyn core::fmt::Debug + core::marker::Send as app::OnArr>::on_arr",
            ),
            (
                "<dyn app::b::DynT>::inherent_on_dyn::__hotpath_site",
                false,
                "app::b::<dyn DynT>::inherent_on_dyn",
                "<dyn app::b::DynT>::inherent_on_dyn",
            ),
            // async fn: the coroutine's `{{closure}}` is not part of the name
            (
                "app::a::Foo::async_m::{{closure}}::__hotpath_site",
                true,
                "app::a::Foo::async_m",
                "app::a::Foo::async_m",
            ),
            // a fn inside a closure keeps the closure segment
            (
                "app::m::two::{{closure}}::helper::__hotpath_site",
                false,
                "app::m::two::{{closure}}::helper",
                "app::m::two::{{closure}}::helper",
            ),
        ];
        for &(type_name, is_async, key, pattern) in cases {
            let identity = site_identity("app::b", type_name, is_async);
            assert_eq!(identity.key, key, "key of {type_name}");
            assert_eq!(identity.pattern, pattern, "pattern of {type_name}");
        }

        // A trait from a private module prints its definition path; the key
        // keeps only the last segment.
        let identity = site_identity(
            "ratatui_widgets::block",
            "<&ratatui_widgets::block::Block as ratatui_core::widgets::widget::Widget>::render::__hotpath_site",
            false,
        );
        assert_eq!(
            identity.key,
            "ratatui_widgets::block::<&Block as Widget>::render"
        );
        assert_eq!(
            identity.pattern,
            "<&ratatui_widgets::block::Block as ratatui_core::widgets::widget::Widget>::render"
        );
    }

    #[test]
    fn unknown_shapes_keep_the_raw_path() {
        let identity = site_identity("app", "app::<weird::m::__hotpath_site", false);
        assert_eq!(identity.key, "app::<weird::m");
        assert_eq!(identity.pattern, "app::<weird::m");
    }
}
