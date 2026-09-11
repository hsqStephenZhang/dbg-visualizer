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
use rustc_span::FileName;
use rustc_trait_selection::infer::InferCtxtExt;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;

struct Driver {
    plan: PathBuf,
    inject: bool,
}

fn write_changed(path: &std::path::Path, contents: &str) {
    if std::fs::read_to_string(path).ok().as_deref() != Some(contents) {
        std::fs::write(path, contents).unwrap();
    }
}

// Resolve actual publicly exported paths, not diagnostic type-name strings.
fn paths(tcx: TyCtxt<'_>) -> HashMap<DefId, String> {
    let mut queue = VecDeque::from([(CRATE_DEF_ID.to_def_id(), "crate".to_string())]);
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

fn render<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    paths: &HashMap<DefId, String>,
) -> Result<String, &'static str> {
    Ok(match ty.kind() {
        ty::Bool => "bool".into(),
        ty::Char => "char".into(),
        ty::Int(i) => i.name_str().into(),
        ty::Uint(i) => i.name_str().into(),
        ty::Float(i) => i.name_str().into(),
        ty::Tuple(fields) => format!(
            "({})",
            fields
                .iter()
                .map(|t| render(tcx, t, paths).map(|s| format!("{s},")))
                .collect::<Result<String, _>>()?
        ),
        ty::Array(t, n) => format!(
            "[{}; {}]",
            render(tcx, *t, paths)?,
            n.try_to_target_usize(tcx).ok_or("non-usize array length")?
        ),
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
                } = param.kind && tcx
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
            for arg in &args[..count] {
                values.push(match arg.kind() {
                    ty::GenericArgKind::Type(t) => render(tcx, t, paths)?,
                    ty::GenericArgKind::Const(c) => c
                        .try_to_target_usize(tcx)
                        .ok_or("unsupported const argument")?
                        .to_string(),
                    ty::GenericArgKind::Lifetime(_) => {
                        return Err("borrowed type: lifetime proof not implemented");
                    }
                });
            }
            if values.is_empty() {
                path.clone()
            } else {
                format!("{path}<{}>", values.join(", "))
            }
        }
        ty::Ref(..) => return Err("borrowed type: lifetime proof not implemented"),
        _ => return Err("unsupported/anonymous/unsized type"),
    })
}

impl Callbacks for Driver {
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
                    && tcx.crate_name(did.krate).as_str() == "visualizer_runtime")
                    .then(|| (*did, path.rsplit_once("::").unwrap().0.to_string()))
            })
            .expect("dbgvis dependency must be used (call enable!())");
        let trait_ids: Vec<_> = paths.keys().filter_map(|did| {
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
        for cgu in tcx.collect_and_partition_mono_items(()).codegen_units {
            for item in cgu.items().keys() {
                let MonoItem::Fn(instance) = item else {
                    continue;
                };
                if instance.def_id().krate != LOCAL_CRATE {
                    continue;
                }
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
                            rustc_middle::mir::VarDebugInfoContents::Const(value) => {
                                value.const_.ty()
                            }
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
        }
        let mut lines = BTreeMap::new();
        let mut report = BTreeMap::new();
        for t in candidates {
            let name = t.to_string(); // Diagnostic only; never used as generated Rust.
            let expression = match render(tcx, t, &paths) {
                Ok(expr) => expr,
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
            report.insert(name, format!("CANDIDATE\t{expression}"));
            lines.insert(
                expression.clone(),
                format!("{}::register_type!({expression});\n", facade.1),
            );
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
    let plan = std::env::var_os("DBGVIS_PLAN")
        .expect("DBGVIS_PLAN required")
        .into();
    let inject = std::env::var_os("DBGVIS_INJECT").is_some();
    let args: Vec<_> = std::env::args().collect();
    rustc_driver::run_compiler(&args, &mut Driver { plan, inject });
}
