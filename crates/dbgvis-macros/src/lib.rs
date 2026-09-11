use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::{
    Attribute, Data, DeriveInput, Fields, Item, ItemFn, LitStr, Type, parse_macro_input,
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
    no_register: bool,
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
            } else if meta.path.is_ident("no_register") {
                if out.no_register {
                    return Err(meta.error("duplicate no_register"));
                }
                out.no_register = true;
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
        if config.transparent || !config.bounds.is_empty() || config.no_register {
            return Err(syn::Error::new(
                field.span(),
                "transparent/bound/no_register are type-level options",
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
                    || variant_config.no_register
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
    let registration = if input.generics.params.is_empty() && !config.no_register {
        emit_registration(
            name,
            &syn::parse_quote!(#name),
            &input.generics,
            &input.attrs,
            &ModeOptions::default(),
            &path,
        )?
    } else {
        Tokens::new()
    };
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics #path::Visualize for #name #ty_generics #where_clause {
            fn visualize(&self, __dbgvis_out: &mut #path::Formatter<'_>) -> #path::Result { #generated }
        }
        #registration
    })
}

#[derive(Default)]
struct ModeOptions {
    modes: Vec<String>,
    default: Option<String>,
}
impl syn::parse::Parse for ModeOptions {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut out = Self::default();
        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            let word = ident.to_string();
            if word == "default" {
                if out.default.is_some() {
                    return Err(syn::Error::new(ident.span(), "duplicate default"));
                }
                input.parse::<syn::Token![=]>()?;
                out.default = Some(input.parse::<LitStr>()?.value());
            } else if ["auto", "visualize", "debug", "display"].contains(&word.as_str()) {
                if out.modes.contains(&word) {
                    return Err(syn::Error::new(ident.span(), "duplicate mode"));
                }
                out.modes.push(word);
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    "expected auto, visualize, debug, display or default",
                ));
            }
            if input.is_empty() {
                break;
            }
            input.parse::<syn::Token![,]>()?;
        }
        Ok(out)
    }
}

fn emit_registration(
    ident: &syn::Ident,
    ty: &Type,
    generics: &syn::Generics,
    attrs: &[Attribute],
    options: &ModeOptions,
    path: &Tokens,
) -> syn::Result<Tokens> {
    if generics.type_params().next().is_some() || generics.const_params().next().is_some() {
        return Err(syn::Error::new(
            generics.span(),
            "register concrete type/const arguments, not a generic type family",
        ));
    }
    let mut modes = options.modes.clone();
    if modes.is_empty() {
        modes.push("auto".into());
    }
    modes.sort_by_key(|m| {
        ["auto", "visualize", "debug", "display"]
            .iter()
            .position(|x| x == m)
            .unwrap()
    });
    let default = options.default.as_ref().unwrap_or(&modes[0]);
    if !modes.contains(default) {
        return Err(syn::Error::new(
            ident.span(),
            "default mode must be registered",
        ));
    }
    let default = format_ident!("{}", default.to_uppercase());
    let calls = modes
        .iter()
        .map(|mode| format_ident!("{mode}"))
        .collect::<Vec<_>>();
    let cfg = attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg"))
        .collect::<Vec<_>>();
    let anchor = format_ident!("__DBG_ANCHOR_{ident}");
    let anchor_type = format_ident!("__DBG_ANCHOR_{ident}Type");
    let marker = format_ident!("__DbgMarker{ident}");
    let factory = format_ident!("__dbgvis_entry_{ident}");
    let element = format_ident!("__DBG_REGISTRATION_{ident}");
    // Substitute only declared lifetime parameters in the DWARF-only anchor.
    // The formatter factory below retains the original generic lifetimes.
    struct StaticLifetimes(Vec<syn::Lifetime>);
    impl syn::visit_mut::VisitMut for StaticLifetimes {
        fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
            if self.0.contains(lifetime) {
                *lifetime = syn::parse_quote!('static);
            }
        }
    }
    let mut anchor_ty = ty.clone();
    syn::visit_mut::VisitMut::visit_type_mut(
        &mut StaticLifetimes(generics.lifetimes().map(|p| p.lifetime.clone()).collect()),
        &mut anchor_ty,
    );
    let where_clause = &generics.where_clause;
    Ok(quote! {
        #(#cfg)* #[doc(hidden)] struct #marker;
        #(#cfg)* #[doc(hidden)] #[repr(C)] #[allow(non_camel_case_types)]
        struct #anchor_type { typed: *const #anchor_ty }
        #(#cfg)* unsafe impl Sync for #anchor_type {}
        #(#cfg)* #[doc(hidden)] #[allow(non_upper_case_globals)]
        static #anchor: #anchor_type = #anchor_type { typed: ::std::ptr::null() };
        #(#cfg)* #[allow(non_snake_case)]
        fn #factory #generics () -> #path::Root #where_clause {
            ::std::hint::black_box(&#anchor);
            // type_name includes function-local scopes, unlike module_path!().
            let anchor = ::std::any::type_name::<#anchor_type>().strip_suffix("Type").unwrap();
            #path::Registration::<#ty>::new::<#marker>(anchor)
                #(.#calls())*.default_mode(#path::#default).finish()
        }
        #(#cfg)*
        #[#path::__linkme::distributed_slice(#path::VIS_TYPES)]
        #[linkme(crate = #path::__linkme)]
        #[allow(non_upper_case_globals)]
        static #element: fn() -> #path::Root = #factory;
    })
}

/// Register a struct/enum or a concrete alias without implementing Visualize.
#[proc_macro_attribute]
pub fn register(args: TokenStream, input: TokenStream) -> TokenStream {
    let options = parse_macro_input!(args as ModeOptions);
    let item = parse_macro_input!(input as Item);
    let result = (|| {
        let (ident, generics, attrs) = match &item {
            Item::Struct(item) => (&item.ident, &item.generics, &item.attrs),
            Item::Enum(item) => (&item.ident, &item.generics, &item.attrs),
            Item::Type(item) => (&item.ident, &item.generics, &item.attrs),
            _ => {
                return Err(syn::Error::new(
                    item.span(),
                    "register expects a struct, enum or concrete type alias",
                ));
            }
        };
        let (_, arguments, _) = generics.split_for_impl();
        let ty = syn::parse_quote!(#ident #arguments);
        let registration = emit_registration(ident, &ty, generics, attrs, &options, &facade())?;
        Ok(quote!(#item #registration))
    })();
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Register a concrete third-party or generic root in item position.
#[proc_macro]
pub fn register_type(input: TokenStream) -> TokenStream {
    struct Input {
        ty: Type,
        options: ModeOptions,
    }
    impl syn::parse::Parse for Input {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let ty = input.parse()?;
            let options = if input.is_empty() {
                ModeOptions::default()
            } else {
                input.parse::<syn::Token![;]>()?;
                input.parse()?
            };
            Ok(Self { ty, options })
        }
    }
    let Input { ty, options } = parse_macro_input!(input as Input);
    // Deterministic, scope-local name; collisions are compile errors, never wrong dispatch.
    let spelling = quote!(#ty).to_string();
    let hash = spelling.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    let ident = format_ident!("Type_{hash:016x}");
    emit_registration(
        &ident,
        &ty,
        &syn::Generics::default(),
        &[],
        &options,
        &facade(),
    )
    .unwrap_or_else(syn::Error::into_compile_error)
    .into()
}

#[proc_macro_attribute]
pub fn main(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "main takes no arguments; registrations are collected by linkme",
        )
        .into_compile_error()
        .into();
    }
    let mut function = parse_macro_input!(input as ItemFn);
    let path = facade();
    function
        .block
        .stmts
        .insert(0, syn::parse_quote!(#path::enable!();));
    quote!(#function).into()
}
