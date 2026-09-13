//! Experimental two-session driver. Built directly with the pinned rustc-dev sysroot.
#![feature(rustc_private)]
extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_infer;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_parse;
extern crate rustc_span;
extern crate rustc_trait_selection;

use rustc_driver::{Callbacks, Compilation};
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{CRATE_DEF_ID, DefId, LOCAL_CRATE};
use rustc_infer::infer::TyCtxtInferExt;
use rustc_interface::interface::Compiler;
use rustc_middle::mono::MonoItem;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::{FileName, Symbol};
use rustc_trait_selection::infer::InferCtxtExt;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;

struct Driver {
    plan: PathBuf,
    inject: bool,
}

// Record wrapper inputs even for workspace libraries that only pass through.
// Cargo must rebuild those libraries to add/remove encoded MIR when the scan
// mode changes; tracking this solely in the injected executable is too late.
fn track_wrapper_inputs(config: &mut rustc_interface::interface::Config) {
    config.track_state = Some(Box::new(|sess| {
        for key in ["DBGVIS_AUTO_CRATE", "DBGVIS_SCAN_DEPS"] {
            sess.env_depinfo.borrow_mut().insert((
                Symbol::intern(key),
                std::env::var(key).ok().as_deref().map(Symbol::intern),
            ));
        }
        let directory = PathBuf::from(std::env::var_os("DBGVIS_TOOL_DIR").expect("tool directory"));
        for path in [
            directory.join("wrapper.py"),
            directory.join("driver.rs"),
            directory.join("../../xtask/src/autoregister.rs"),
        ] {
            sess.file_depinfo
                .borrow_mut()
                .insert(Symbol::intern(path.to_str().expect("UTF-8 tool path")));
        }
    }));
}

struct Passthrough;

impl Callbacks for Passthrough {
    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        track_wrapper_inputs(config);
    }
}

fn write_changed(path: &std::path::Path, contents: &str) {
    if std::fs::read_to_string(path).ok().as_deref() != Some(contents) {
        std::fs::write(path, contents).unwrap();
    }
}

// Resolve actual publicly exported paths, not diagnostic type-name strings.
fn paths(tcx: TyCtxt<'_>) -> HashMap<DefId, String> {
    // Visit external crates before the local root.  A local `use dbgvis::Visualize`
    // re-export otherwise wins the map entry and produces `crate::register_type!`
    // instead of the actual facade path.
    let mut queue = VecDeque::new();
    for &cnum in tcx.crates(()) {
        let source = tcx.used_crate_source(cnum);
        for (alias, entry) in tcx.sess.opts.externs.iter() {
            if entry.files().is_some_and(|mut files| {
                files.any(|file| {
                    [&source.rlib, &source.rmeta, &source.dylib]
                        .into_iter()
                        .flatten()
                        .any(
                            |path| match (path.canonicalize(), file.original().canonicalize()) {
                                (Ok(actual), Ok(extern_path)) => {
                                    actual == extern_path
                                        || ([Some("rlib"), Some("rmeta")]
                                            .contains(&actual.extension().and_then(|s| s.to_str()))
                                            && [Some("rlib"), Some("rmeta")].contains(
                                                &extern_path.extension().and_then(|s| s.to_str()),
                                            )
                                            && actual.with_extension("rmeta")
                                                == extern_path.with_extension("rmeta"))
                                }
                                _ => false,
                            },
                        )
                })
            }) {
                queue.push_back((cnum.as_def_id(), format!("::{alias}")));
            }
        }
        if tcx.crate_name(cnum).as_str() == "std" {
            queue.push_back((cnum.as_def_id(), "::std".into()));
        }
    }
    queue.push_back((CRATE_DEF_ID.to_def_id(), "crate".to_string()));
    let mut result = HashMap::new();
    let mut visited = HashSet::new();
    while let Some((module, prefix)) = queue.pop_front() {
        if !visited.insert(module) {
            continue;
        }
        let children = if let Some(local) = module.as_local() {
            tcx.module_children_local(local)
        } else {
            tcx.module_children(module)
        };
        for child in children {
            let Res::Def(kind, did) = child.res else {
                continue;
            };
            // The generated module is a child of the crate root, not of private submodules.
            if !child.vis.is_accessible_from(CRATE_DEF_ID, tcx) {
                continue;
            }
            let name = child.ident.name.as_str();
            if name.starts_with('_') || name == "self" || name == "super" {
                continue;
            }
            let path = format!("{prefix}::r#{name}");
            if kind == DefKind::Mod {
                queue.push_back((did, path));
            } else {
                result.entry(did).or_insert(path);
            }
        }
    }
    result
}

/// Non-generic functions of the dependency crates named in `DBGVIS_SCAN_CRATES`.
///
/// Upstream non-generic functions are codegened in their own crate, so they never
/// appear in this crate's mono items and their locals are invisible to the scan.
/// Enumerating them from the module tree recovers those locals, but only works when
/// the crate carries MIR, which the wrapper arranges with `-Zalways-encode-mir`.
///
/// The crate names are explicit rather than inferred from MIR availability: the
/// distributed `std` and `core` also encode MIR, and scanning them would turn every
/// standard-library local into a registered root.
fn dependency_functions<'tcx>(tcx: TyCtxt<'tcx>) -> Vec<ty::Instance<'tcx>> {
    let Ok(selected) = std::env::var("DBGVIS_SCAN_CRATES") else {
        return Vec::new();
    };
    let selected: HashSet<&str> = selected.split(',').filter(|s| !s.is_empty()).collect();
    if selected.is_empty() {
        return Vec::new();
    }
    let mut queue: VecDeque<DefId> = VecDeque::new();
    for &cnum in tcx.crates(()) {
        if selected.contains(tcx.crate_name(cnum).as_str()) {
            queue.push_back(cnum.as_def_id());
        }
    }
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    // Only a function with no generic parameters has a single instantiation the scan
    // can name; a generic one has no concrete arguments to substitute here.
    fn mono<'tcx>(tcx: TyCtxt<'tcx>, did: DefId) -> Option<ty::Instance<'tcx>> {
        (tcx.generics_of(did).count() == 0 && tcx.is_mir_available(did))
            .then(|| ty::Instance::mono(tcx, did))
    }
    while let Some(module) = queue.pop_front() {
        if !visited.insert(module) {
            continue;
        }
        // Stay inside the selected crates: a `pub use` can re-export another crate's
        // module, and following it would drag in the whole standard library.
        if !selected.contains(tcx.crate_name(module.krate).as_str()) {
            continue;
        }
        for child in tcx.module_children(module) {
            let Res::Def(kind, did) = child.res else {
                continue;
            };
            match kind {
                DefKind::Mod => queue.push_back(did),
                DefKind::Fn => result.extend(mono(tcx, did)),
                // Inherent methods are not module children; reach them through the type.
                DefKind::Struct | DefKind::Enum | DefKind::Union => {
                    for implementation in tcx.inherent_impls(did) {
                        for assoc in tcx.associated_item_def_ids(*implementation) {
                            if tcx.def_kind(*assoc) == DefKind::AssocFn {
                                result.extend(mono(tcx, *assoc));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    result
}

struct Rendered {
    expr: String,
    borrowed: bool,
}

fn render<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    paths: &HashMap<DefId, String>,
) -> Result<Rendered, &'static str> {
    Ok(match ty.kind() {
        ty::Bool => Rendered {
            expr: "bool".into(),
            borrowed: false,
        },
        ty::Char => Rendered {
            expr: "char".into(),
            borrowed: false,
        },
        ty::Int(i) => Rendered {
            expr: i.name_str().into(),
            borrowed: false,
        },
        ty::Uint(i) => Rendered {
            expr: i.name_str().into(),
            borrowed: false,
        },
        ty::Float(i) => Rendered {
            expr: i.name_str().into(),
            borrowed: false,
        },
        ty::Str => Rendered {
            expr: "str".into(),
            borrowed: false,
        },
        ty::Tuple(fields) => {
            let mut borrowed = false;
            let mut values = String::new();
            for field in fields.iter() {
                let rendered = render(tcx, field, paths)?;
                borrowed |= rendered.borrowed;
                values.push_str(&rendered.expr);
                values.push(',');
            }
            Rendered {
                expr: format!("({values})"),
                borrowed,
            }
        }
        ty::Array(t, n) => {
            let rendered = render(tcx, *t, paths)?;
            Rendered {
                expr: format!(
                    "[{}; {}]",
                    rendered.expr,
                    n.try_to_target_usize(tcx).ok_or("non-usize array length")?
                ),
                borrowed: rendered.borrowed,
            }
        }
        ty::Adt(adt, args) => {
            let path = paths
                .get(&adt.did())
                .ok_or("type is not accessible/nameable from root")?;
            let params = &tcx.generics_of(adt.did()).own_params;
            let mut count = args.len();
            // Omit only trailing defaults equal to the actual instantiated arguments.
            while count > 0 {
                let param = &params[count - 1];
                if let ty::GenericParamDefKind::Type {
                    has_default: true, ..
                } = param.kind
                    && tcx
                        .type_of(param.def_id)
                        .instantiate(tcx, args)
                        .skip_norm_wip()
                        == args[count - 1].expect_ty()
                {
                    count -= 1;
                    continue;
                }
                break;
            }
            let mut values = Vec::new();
            let mut borrowed = false;
            for arg in &args[..count] {
                values.push(match arg.kind() {
                    ty::GenericArgKind::Type(t) => {
                        let rendered = render(tcx, t, paths)?;
                        borrowed |= rendered.borrowed;
                        rendered.expr
                    }
                    ty::GenericArgKind::Const(c) => c
                        .try_to_target_usize(tcx)
                        .ok_or("unsupported const argument")?
                        .to_string(),
                    // Supported borrowed candidates use a synthetic lifetime;
                    // register substitutes 'static in the anchor/identity type
                    // while retaining the generic formatter.
                    ty::GenericArgKind::Lifetime(_) => {
                        borrowed = true;
                        "'a".into()
                    }
                });
            }
            let expr = if values.is_empty() {
                path.clone()
            } else {
                format!("{path}<{}>", values.join(", "))
            };
            Rendered { expr, borrowed }
        }
        ty::Ref(_, inner, mutability) => {
            let rendered = render(tcx, *inner, paths)?;
            let mutability = if *mutability == ty::Mutability::Mut {
                "mut "
            } else {
                ""
            };
            Rendered {
                expr: format!("&'a {mutability}{}", rendered.expr),
                borrowed: true,
            }
        }
        _ => return Err("unsupported/anonymous/unsized type"),
    })
}

impl Callbacks for Driver {
    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        track_wrapper_inputs(config);
    }

    fn after_crate_root_parsing(
        &mut self,
        compiler: &Compiler,
        krate: &mut rustc_ast::Crate,
    ) -> Compilation {
        if self.inject {
            let tool_dir = std::env::var("DBGVIS_TOOL_DIR").expect("DBGVIS_TOOL_DIR required");
            // Let Cargo track tool/config changes as inputs to the real compilation.
            let source = format!(
                "#[allow(dead_code, unused_imports)] mod __dbgvis_generated {{
                    include!({:?});
                    const _: &str = include!(concat!({tool_dir:?}, \"/tracked.rs\"));
                }}",
                self.plan
            );
            let mut parser = rustc_parse::new_parser_from_source_str(
                &compiler.sess.psess,
                FileName::Custom("dbgvis injection".into()),
                source,
                rustc_parse::lexer::StripTokens::Nothing,
            )
            .unwrap_or_else(|errors| {
                for e in errors {
                    e.emit();
                }
                panic!("injection parse failed")
            });
            let item = parser
                .parse_item(
                    rustc_parse::parser::ForceCollect::No,
                    rustc_parse::parser::AllowConstBlockItems::No,
                )
                .unwrap_or_else(|e| {
                    e.emit();
                    panic!("injection item failed")
                })
                .unwrap();
            krate.items.push(item);
        }
        Compilation::Continue
    }

    fn after_analysis<'tcx>(&mut self, _compiler: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        if self.inject {
            return Compilation::Continue;
        }
        let paths = paths(tcx);
        let facade = paths
            .iter()
            .find_map(|(did, path)| {
                (tcx.item_name(*did).as_str() == "Visualize"
                    && tcx.def_kind(*did) == DefKind::Trait
                    && tcx.crate_name(did.krate).as_str() == "dbgvis_runtime")
                    .then(|| (*did, path.rsplit_once("::").unwrap().0.to_string()))
            })
            .expect("dbgvis dependency must be used (call enable!())");
        let trait_ids: Vec<_> = paths
            .keys()
            .filter_map(|did| {
                (tcx.def_kind(*did) == DefKind::Trait
                    && (*did == facade.0
                        || (tcx.crate_name(did.krate).as_str() == "core"
                            && ["Debug", "Display"].contains(&tcx.item_name(*did).as_str()))))
                .then_some(*did)
            })
            .collect();
        let (infcx, param_env) = tcx
            .infer_ctxt()
            .build_with_typing_env(ty::TypingEnv::fully_monomorphized());
        let mut existing = HashSet::new();
        // Existing macros retain typed anchors. Read their types, not diagnostic strings.
        for did in tcx.iter_local_def_id() {
            if matches!(tcx.def_kind(did), DefKind::Static { .. })
                && tcx
                    .item_name(did.to_def_id())
                    .as_str()
                    .starts_with("__DBG_ANCHOR_")
            {
                let anchor = tcx.type_of(did).instantiate_identity().skip_norm_wip();
                if let ty::Adt(adt, args) = anchor.kind() {
                    for field in adt.all_fields() {
                        if let ty::RawPtr(t, _) = field.ty(tcx, args).skip_norm_wip().kind() {
                            existing.insert(*t);
                        }
                    }
                }
            }
        }

        // collect types for visualizer code generation
        let mut candidates = HashSet::new();
        let mut instances: Vec<ty::Instance<'_>> = Vec::new();
        for cgu in tcx.collect_and_partition_mono_items(()).codegen_units {
            for item in cgu.items().keys() {
                let MonoItem::Fn(instance) = item else {
                    continue;
                };
                if instance.def_id().krate != LOCAL_CRATE {
                    continue;
                }
                instances.push(*instance);
            }
        }
        // Named variables in dependency crates. A non-generic function of an upstream
        // crate is codegened there, not here, so it never appears in this crate's
        // mono items: it has to be enumerated from the crate's module tree instead,
        // and its MIR only exists because the wrapper built that crate with
        // `-Zalways-encode-mir`. The crate list is explicit because the distributed
        // std/core encode MIR too, and scanning those would register thousands of
        // standard-library locals.
        instances.extend(dependency_functions(tcx));
        for instance in instances {
            let name = tcx.def_path_str(instance.def_id());
            if name.contains("__dbgvis") || name.contains("__Dbg") {
                continue;
            }
            let body = tcx.instance_mir(instance.def);
            for variable in &body.var_debug_info {
                if variable.source_info.span.from_expansion() {
                    continue;
                }
                let scope = &body.source_scopes[variable.source_info.scope];
                if scope.inlined.is_some() || scope.inlined_parent_scope.is_some() {
                    continue;
                }
                let local_ty = if let Some(fragment) = &variable.composite {
                    fragment.ty
                } else {
                    match &variable.value {
                        rustc_middle::mir::VarDebugInfoContents::Place(place) => {
                            place.ty(&body.local_decls, tcx).ty
                        }
                        rustc_middle::mir::VarDebugInfoContents::Const(value) => value.const_.ty(),
                    }
                };
                let t = instance.instantiate_mir_and_normalize_erasing_regions(
                    tcx,
                    ty::TypingEnv::fully_monomorphized(),
                    ty::EarlyBinder::bind(tcx, local_ty),
                );
                candidates.insert(t);
            }
        }
        let mut lines = BTreeMap::new();
        let mut report = BTreeMap::new();
        for t in candidates {
            // MIR contains many compiler-generated/reference temporaries (for
            // example the argument passed to black_box). Register the actual
            // value type instead; a reference's unconditional Visualize impl
            // would otherwise defer validation and generate a failing root for
            // an unformattable referent.
            if matches!(t.kind(), ty::Ref(..)) {
                continue;
            }
            let name = t.to_string(); // Diagnostic only; never used as generated Rust.
            let rendered = match render(tcx, t, &paths) {
                Ok(rendered) => rendered,
                Err(reason) => {
                    report.insert(name, format!("SKIP\t{reason}"));
                    continue;
                }
            };
            if existing.contains(&t) {
                report.insert(name, "EXISTING\tlocal typed anchor".into());
                continue;
            }
            if !trait_ids.iter().any(|did| {
                infcx
                    .type_implements_trait(*did, [t], param_env)
                    .must_apply_modulo_regions()
            }) {
                report.insert(name, "SKIP\tno formatting trait".into());
                continue;
            }
            report.insert(name, format!("CANDIDATE\t{}", rendered.expr));
            if rendered.borrowed {
                // A generic alias lets the existing proc-macro preserve the
                // formatter's lifetime while giving GDB a concrete anchor.
                let hash = rendered.expr.bytes().fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ b as u64).wrapping_mul(0x100000001b3)
                });
                lines.insert(
                    rendered.expr.clone(),
                    format!(
                        "#[allow(non_camel_case_types)]\n#[{}::register]\ntype __DbgvisAuto_{hash:016x}<'a> = {};\n",
                        facade.1, rendered.expr
                    ),
                );
            } else {
                lines.insert(
                    rendered.expr.clone(),
                    format!("{}::register_type!({});\n", facade.1, rendered.expr),
                );
            }
        }
        assert!(lines.len() <= 4096, "dbgvis auto: candidate limit exceeded");
        write_changed(&self.plan, &lines.into_values().collect::<String>());
        write_changed(
            &self.plan.with_extension("tsv"),
            &report
                .into_iter()
                .map(|(t, status)| format!("{status}\t{t}\n"))
                .collect::<String>(),
        );
        Compilation::Stop
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if std::env::var_os("DBGVIS_PASSTHROUGH").is_some() {
        rustc_driver::run_compiler(&args, &mut Passthrough);
        return;
    }
    let plan = std::env::var_os("DBGVIS_PLAN")
        .expect("DBGVIS_PLAN required")
        .into();
    let inject = std::env::var_os("DBGVIS_INJECT").is_some();
    rustc_driver::run_compiler(&args, &mut Driver { plan, inject });
}
