use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::{
    Attribute, Data, DeriveInput, Fields, Item, ItemFn, ItemMod, LitStr, Type, parse_macro_input,
    spanned::Spanned,
};

// Resolve the facade's Cargo dependency alias, including workspace inheritance.
fn facade() -> Tokens {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from);
    if let Some(directory) = manifest
        && let Ok(source) = std::fs::read_to_string(directory.join("Cargo.toml"))
        && let Ok(doc) = source.parse::<toml::Table>()
    {
        for section in ["dependencies", "dev-dependencies"] {
            if let Some(table) = doc.get(section).and_then(toml::Value::as_table) {
                for (alias, spec) in table {
                    let mut package = spec
                        .get("package")
                        .and_then(toml::Value::as_str)
                        .unwrap_or(alias)
                        .to_owned();
                    if spec.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                        for ancestor in directory.ancestors() {
                            let inherited = std::fs::read_to_string(ancestor.join("Cargo.toml"))
                                .ok()
                                .and_then(|s| s.parse::<toml::Table>().ok());
                            if let Some(name) = inherited
                                .as_ref()
                                .and_then(|d| d.get("workspace"))
                                .and_then(|w| w.get("dependencies"))
                                .and_then(|d| d.get(alias))
                                .and_then(|d| d.get("package"))
                                .and_then(toml::Value::as_str)
                            {
                                package = name.to_owned();
                                break;
                            }
                        }
                    }
                    if package == "dbgvis" {
                        let ident = format_ident!("{}", alias.replace('-', "_"));
                        return quote!(::#ident);
                    }
                }
            }
        }
    }
    quote!(::dbgvis)
}

#[derive(Default)]
struct Options {
    skip: bool,
    rename: Option<String>,
    via: Option<String>,
    transparent: bool,
    bounds: Vec<syn::WherePredicate>,
}
fn options(attrs: &[Attribute]) -> syn::Result<Options> {
    let mut out = Options::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("dbgvis")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                if out.skip {
                    return Err(meta.error("duplicate skip"));
                }
                out.skip = true;
            } else if meta.path.is_ident("transparent") {
                if out.transparent {
                    return Err(meta.error("duplicate transparent"));
                }
                out.transparent = true;
            } else if meta.path.is_ident("rename") {
                if out.rename.is_some() {
                    return Err(meta.error("duplicate rename"));
                }
                out.rename = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("via") {
                if out.via.is_some() {
                    return Err(meta.error("duplicate via"));
                }
                let value = meta.value()?.parse::<LitStr>()?;
                if !matches!(value.value().as_str(), "debug" | "display") {
                    return Err(meta.error("via must be debug or display"));
                }
                out.via = Some(value.value());
            } else if meta.path.is_ident("bound") {
                let text = meta.value()?.parse::<LitStr>()?.value();
                let clause: syn::WhereClause = syn::parse_str(&format!("where {text}"))?;
                out.bounds.extend(clause.predicates);
            } else {
                return Err(meta.error("unsupported dbgvis option"));
            }
            Ok(())
        })?;
    }
    if out.skip && (out.via.is_some() || out.rename.is_some() || out.transparent) {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "skip cannot be combined with formatting options",
        ));
    }
    Ok(out)
}

fn body(
    fields: &Fields,
    label: &str,
    values: &[Tokens],
    transparent: bool,
    path: &Tokens,
    bounds: &mut Vec<syn::WherePredicate>,
) -> syn::Result<Tokens> {
    let mut active = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        let config = options(&field.attrs)?;
        if config.transparent || !config.bounds.is_empty() {
            return Err(syn::Error::new(
                field.span(),
                "transparent/bound are type-level options",
            ));
        }
        if config.skip {
            continue;
        }
        if let Some(ref via) = config.via {
            let typ = &field.ty;
            let bound: syn::WherePredicate = if via == "debug" {
                syn::parse_quote!(#typ: ::std::fmt::Debug)
            } else {
                syn::parse_quote!(#typ: ::std::fmt::Display)
            };
            bounds.push(bound);
        }
        active.push((index, field, config));
    }
    if transparent {
        if active.len() != 1 {
            return Err(syn::Error::new(
                fields.span(),
                "transparent requires exactly one non-skipped field",
            ));
        }
        let (index, _, config) = &active[0];
        let method = format_ident!("{}", config.via.as_deref().unwrap_or("value"));
        let value = &values[*index];
        return Ok(quote!(__dbgvis_out.#method(#value)));
    }
    if matches!(fields, Fields::Unit) {
        return Ok(quote!(::std::fmt::Write::write_str(__dbgvis_out, #label)));
    }
    let named = matches!(fields, Fields::Named(_));
    let constructor = if named {
        format_ident!("record")
    } else {
        format_ident!("tuple")
    };
    let mut steps = Vec::new();
    for (index, field, config) in active {
        let value = &values[index];
        let suffix = config.via.map(|s| format!("_{s}")).unwrap_or_default();
        let method = format_ident!("{}{}", if named { "field" } else { "item" }, suffix);
        if named {
            let name = config
                .rename
                .unwrap_or_else(|| field.ident.as_ref().unwrap().to_string());
            steps.push(quote!(__dbgvis_group.#method(#name, #value)?;));
        } else {
            steps.push(quote!(__dbgvis_group.#method(#value)?;));
        }
    }
    let _ = path;
    Ok(
        quote!({ let mut __dbgvis_group = __dbgvis_out.#constructor(#label)?; #(#steps)* __dbgvis_group.finish() }),
    )
}

#[proc_macro_derive(Visualize, attributes(dbgvis))]
pub fn derive_visualize(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_impl(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
fn derive_impl(mut input: DeriveInput) -> syn::Result<Tokens> {
    let path = facade();
    let config = options(&input.attrs)?;
    if config.skip || config.via.is_some() {
        return Err(syn::Error::new(
            input.span(),
            "skip/via are field-level options",
        ));
    }
    let name = &input.ident;
    let label = config.rename.unwrap_or_else(|| name.to_string());
    let mut bounds = config.bounds;
    let generated = match &input.data {
        Data::Struct(data) => {
            let values = data
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let member = field
                        .ident
                        .clone()
                        .map(syn::Member::Named)
                        .unwrap_or(syn::Member::Unnamed(syn::Index::from(index)));
                    quote!(&self.#member)
                })
                .collect::<Vec<_>>();
            body(
                &data.fields,
                &label,
                &values,
                config.transparent,
                &path,
                &mut bounds,
            )?
        }
        Data::Enum(data) => {
            if config.transparent {
                return Err(syn::Error::new(
                    input.span(),
                    "transparent enums are not supported",
                ));
            }
            let mut arms = Vec::new();
            for variant in &data.variants {
                let variant_config = options(&variant.attrs)?;
                if variant_config.skip
                    || variant_config.via.is_some()
                    || variant_config.transparent
                    || !variant_config.bounds.is_empty()
                {
                    return Err(syn::Error::new(
                        variant.span(),
                        "variants only support rename",
                    ));
                }
                let ident = &variant.ident;
                let label = format!(
                    "{}::{}",
                    label,
                    variant_config.rename.unwrap_or_else(|| ident.to_string())
                );
                let bindings = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format_ident!("__dbgvis_field_{i}"))
                    .collect::<Vec<_>>();
                let patterns = variant
                    .fields
                    .iter()
                    .zip(&bindings)
                    .map(|(field, binding)| {
                        let binding = if options(&field.attrs).map(|o| o.skip).unwrap_or(false) {
                            quote!(_)
                        } else {
                            quote!(#binding)
                        };
                        if let Some(ident) = &field.ident {
                            quote!(#ident: #binding)
                        } else {
                            binding
                        }
                    })
                    .collect::<Vec<_>>();
                let pattern = match variant.fields {
                    Fields::Named(_) => quote!(Self::#ident { #(#patterns),* }),
                    Fields::Unnamed(_) => quote!(Self::#ident (#(#patterns),*)),
                    Fields::Unit => quote!(Self::#ident),
                };
                let values = bindings.iter().map(|b| quote!(#b)).collect::<Vec<_>>();
                let body = body(&variant.fields, &label, &values, false, &path, &mut bounds)?;
                arms.push(quote!(#pattern => #body));
            }
            quote!(match self { #(#arms),* })
        }
        Data::Union(_) => {
            return Err(syn::Error::new(
                input.span(),
                "Visualize cannot inspect an untagged union",
            ));
        }
    };
    input.generics.make_where_clause().predicates.extend(bounds);
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics #path::Visualize for #name #ty_generics #where_clause {
            fn visualize(&self, __dbgvis_out: &mut #path::Formatter<'_>) -> #path::Result { #generated }
        }
    })
}

#[proc_macro_attribute]
pub fn visualizers(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "visualizers takes no arguments",
        )
        .into_compile_error()
        .into();
    }
    let input = parse_macro_input!(input as ItemMod);
    registry_impl(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
fn registry_impl(mut module: ItemMod) -> syn::Result<Tokens> {
    let path = facade();
    let (_, items) = module.content.as_mut().ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "visualizers requires an inline module",
        )
    })?;
    let mut extras = Vec::new();
    let mut roots = Vec::new();
    for item in items.iter_mut() {
        let Item::Type(alias) = item else {
            continue;
        };
        let mut modes = Vec::<String>::new();
        let mut default_mode = None;
        for attr in alias.attrs.iter().filter(|a| a.path().is_ident("dbgvis")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("default") {
                    if default_mode.is_some() {
                        return Err(meta.error("duplicate default"));
                    }
                    default_mode = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if let Some(ident) = meta.path.get_ident() {
                    let mode = ident.to_string();
                    if !["auto", "visualize", "debug", "display"].contains(&mode.as_str()) {
                        return Err(meta.error("expected auto, visualize, debug or display"));
                    }
                    if modes.contains(&mode) {
                        return Err(meta.error("duplicate mode"));
                    }
                    modes.push(mode);
                } else {
                    return Err(meta.error("invalid registration option"));
                }
                Ok(())
            })?;
        }
        alias.attrs.retain(|a| !a.path().is_ident("dbgvis"));
        if modes.is_empty() {
            modes.push("auto".into());
        }
        modes.sort_by_key(|m| {
            ["auto", "visualize", "debug", "display"]
                .iter()
                .position(|x| x == m)
                .unwrap()
        });
        if alias.generics.type_params().next().is_some()
            || alias.generics.const_params().next().is_some()
        {
            return Err(syn::Error::new(
                alias.span(),
                "register concrete type/const arguments, not a generic type family",
            ));
        }
        let ident = &alias.ident;
        let cfg = alias
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("cfg"))
            .collect::<Vec<_>>();
        let anchor_ident = format_ident!("__DBG_ANCHOR_{ident}");
        let anchor_type = format_ident!("__DbgAnchor{ident}");
        let marker = format_ident!("__DbgMarker{ident}");
        let factory = format_ident!("__dbgvis_entry_{ident}");
        let lifetime_args = alias
            .generics
            .lifetimes()
            .map(|_| quote!('static))
            .collect::<Vec<_>>();
        let ty: Type = if lifetime_args.is_empty() {
            syn::parse_quote!(#ident)
        } else {
            syn::parse_quote!(#ident<#(#lifetime_args),*>)
        };
        let lifetimes = alias
            .generics
            .lifetimes()
            .map(|p| &p.lifetime)
            .collect::<Vec<_>>();
        let callback_ty: Type = if lifetimes.is_empty() {
            syn::parse_quote!(#ident)
        } else {
            syn::parse_quote!(#ident<#(#lifetimes),*>)
        };
        let generics = &alias.generics;
        let where_clause = &generics.where_clause;
        extras.push(quote! {
            #(#cfg)* #[doc(hidden)] struct #marker;
            #(#cfg)* #[doc(hidden)] #[repr(C)] pub struct #anchor_type { pub typed: *const #ty }
            #(#cfg)* unsafe impl Sync for #anchor_type {}
            #(#cfg)* #[doc(hidden)] pub static #anchor_ident: #anchor_type = #anchor_type { typed: ::std::ptr::null() };
        });
        // Typed callback binds the target independently of its wire registration marker.
        let calls = modes
            .iter()
            .map(|m| format_ident!("{m}"))
            .collect::<Vec<_>>();
        let mut entry = quote!(#path::Registration::<#callback_ty>::new::<#marker>(concat!(module_path!(), "::", stringify!(#anchor_ident))) #(.#calls())*);
        if let Some(mode) = default_mode {
            if !modes.contains(&mode) {
                return Err(syn::Error::new(
                    alias.span(),
                    "default mode must be registered",
                ));
            }
            let constant = format_ident!("{}", mode.to_uppercase());
            entry = quote!(#entry.default_mode(#path::#constant));
        }
        extras.push(quote! {
            #(#cfg)*
            #[allow(non_snake_case)]
            fn #factory #generics () -> #path::Root #where_clause {
                ::std::hint::black_box(&#anchor_ident);
                #entry.finish()
            }
        });
        roots.push(quote!(#(#cfg)* { __dbgvis_roots.push(#factory()); }));
    }
    if roots.is_empty() {
        return Err(syn::Error::new(
            module.span(),
            "register at least one concrete root type",
        ));
    }
    items.push(syn::parse_quote! {
        #[doc(hidden)] #[allow(clippy::vec_init_then_push)]
        pub fn __dbgvis_roots() -> ::std::vec::Vec<#path::Root> {
            let mut __dbgvis_roots = ::std::vec::Vec::new();
            #(#roots)*
            __dbgvis_roots
        }
    });
    let (brace, items) = module.content.take().unwrap();
    let attrs = &module.attrs;
    let vis = &module.vis;
    let name = &module.ident;
    let _ = brace;
    Ok(quote!(#(#attrs)* #vis mod #name { #(#items)* #(#extras)* }))
}

#[proc_macro_attribute]
pub fn main(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::MetaNameValue);
    let mut function = parse_macro_input!(input as ItemFn);
    if !args.path.is_ident("registry") {
        return syn::Error::new(args.span(), "expected registry = module_path")
            .into_compile_error()
            .into();
    }
    let registry = args.value;
    let path = facade();
    function
        .block
        .stmts
        .insert(0, syn::parse_quote!(#path::enable!(#registry);));
    quote!(#function).into()
}
