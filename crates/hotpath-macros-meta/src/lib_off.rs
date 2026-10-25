use proc_macro::TokenStream;

/// No-op version of `#[hotpath_meta::main]` when profiling is disabled.
pub fn main_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// No-op version of `#[hotpath_meta::measure]` when profiling is disabled.
pub fn measure_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// No-op version of `#[hotpath_meta::future_fn]` when profiling is disabled.
pub fn future_fn_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// No-op version of `#[hotpath_meta::skip]` when profiling is disabled.
pub fn skip_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// No-op version of `#[hotpath_meta::measure_all]` when profiling is disabled.
pub fn measure_all_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// No-op version of `hotpath_meta::__unique_label!` when profiling is disabled.
pub fn unique_label_impl(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}

/// No-op version of `hotpath_meta::__callgrind_block!`: the body after the label.
pub fn callgrind_block_impl(input: TokenStream) -> TokenStream {
    crate::split_block_input(input).1
}
