use std::sync::Arc;

use tsrs_core::context::Context;
use tsrs_core::{CompilerOptions, ModuleDetectionKind, ModuleKind, P, ScriptTarget, Tristate};
use tsrs_ls::autoimport::{ProjectID, RegistryExt as _};
use tsrs_ls::{Host as _, new_language_service};
use tsrs_lsproto as lsproto;
use tsrs_vfs::{FS, vfstest};

use crate::{
    APICreateProgramRequest, APISnapshotRequest, FileChangeSummary, SessionInit, SessionOptions,
    Snapshot, new_snapshot_host,
};

struct SnapshotLease(Arc<Snapshot>);

impl Drop for SnapshotLease {
    fn drop(&mut self) {
        self.0.deref();
    }
}

#[test]
fn synthetic_project_native_completions_prepare_auto_imports() {
    let region = tsrs_core::arena::Region::new(64 << 10);
    let _scope = region.enter();
    let context = Context::background();
    let files = [
        ("/src/index.ts", "const local = 1;\n"),
        ("/src/dep.ts", "export const externalValue = 1;"),
        (
            "/src/tsconfig.json",
            r#"{"compilerOptions":{"noLib":true},"files":["index.ts","dep.ts"]}"#,
        ),
    ];
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(files, true));
    let init = SessionInit {
        background_ctx: context.clone(),
        options: Arc::new(SessionOptions {
            current_directory: "/".to_string(),
            position_encoding: lsproto::PositionEncodingKind::UTF16,
            ..Default::default()
        }),
        fs,
        client: None,
        logger: None,
        npm_executor: None,
        parse_cache: None,
        content_mapped_parse_cache: None,
    };
    let host = new_snapshot_host(&init);
    let root = SnapshotLease(host.new_root_snapshot());
    let compiler_options = CompilerOptions {
        module: ModuleKind::ESNext,
        module_detection: ModuleDetectionKind::Force,
        no_lib: Tristate::True,
        target: ScriptTarget::ESNext,
        types: Some(Vec::new()),
        ..Default::default()
    };
    let program = APICreateProgramRequest {
        root_file_names: files[..2].iter().map(|(name, _)| (*name).to_string()).collect(),
        compiler_options: Some(P::new(compiler_options)),
        project_references: Vec::new(),
        config_file_parsing_diagnostics: Vec::new(),
        module_resolver_factory: None,
        module_resolver_id: 0,
    };
    let request = APISnapshotRequest {
        create_programs: vec![program],
        ensure_all_programs: true,
        ..Default::default()
    };
    let snapshot = SnapshotLease(
        host.clone_snapshot(
            &context,
            &root.0,
            FileChangeSummary::default(),
            Some(Arc::new(request)),
        )
        .unwrap_or_else(|(_, error)| panic!("{error:?}")),
    );
    let uri = lsproto::DocumentUri("file:///src/index.ts".to_string());
    assert!(snapshot.0.get_default_project(&uri).is_none());
    let prepared =
        SnapshotLease(host.clone_snapshot_with_auto_imports(&context, &snapshot.0, &uri, None));
    assert!(prepared.0.get_default_project(&uri).is_none());
    let project = prepared.0.project_collection.synthetic_projects().into_iter().next().unwrap();
    let project_id = ProjectID(project.id_string());
    let preferences = prepared.0.get_preferences("/src/index.ts");
    assert!(prepared.0.auto_import_registry().is_prepared_for_importing_file(
        "/src/index.ts",
        &project_id,
        &preferences
    ));
    let program = project.get_program().unwrap();
    let service = new_language_service(
        project_id,
        program,
        Arc::<Snapshot>::clone(&prepared.0),
        "/src/index.ts",
    );
    let position = lsproto::Position { line: 1, character: 0 };
    let output = service.provide_completion(&context, &uri, position, None).unwrap();
    let items = output.items.or_else(|| output.list.map(|list| list.items)).unwrap();
    assert!(items.iter().any(|item| item.label == "local"));
    let item = items.into_iter().find(|item| item.label == "externalValue").unwrap();
    assert!(item.data.as_ref().unwrap().auto_import.is_some());
    let data = item.data.clone();
    let item = service.resolve_completion_item(&context, item, data.as_ref()).unwrap();
    assert_ne!(item.additional_text_edits.unwrap(), Vec::new());

    // Preserve an ordinary default project when the same file also belongs to a synthetic program.
    let request = APISnapshotRequest {
        open_files: Some(
            std::iter::once((
                tsrs_core::tspath::Path::from("/src/index.ts"),
                "/src/index.ts".to_string(),
            ))
            .collect(),
        ),
        ensure_all_programs: true,
        ..Default::default()
    };
    let ordinary = SnapshotLease(
        host.clone_snapshot(
            &context,
            &snapshot.0,
            FileChangeSummary::default(),
            Some(Arc::new(request)),
        )
        .unwrap_or_else(|(_, error)| panic!("{error:?}")),
    );
    let ordinary_project = ordinary.0.get_default_project(&uri).unwrap();
    let ordinary_id = ProjectID(ordinary_project.id_string());
    let prepared_ordinary =
        SnapshotLease(host.clone_snapshot_with_auto_imports(&context, &ordinary.0, &uri, None));
    assert_eq!(prepared_ordinary.0.get_default_project(&uri).unwrap().id(), ordinary_project.id());
    assert!(prepared_ordinary.0.auto_import_registry().is_prepared_for_importing_file(
        "/src/index.ts",
        &ordinary_id,
        &prepared_ordinary.0.get_preferences("/src/index.ts")
    ));
}
