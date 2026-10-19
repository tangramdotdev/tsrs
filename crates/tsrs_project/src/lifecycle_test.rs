use std::sync::Arc;

use tsrs_core::arena::Region;
use tsrs_core::context::Context;
use tsrs_core::{CompilerOptions, ModuleKind, P, ScriptTarget, Tristate};
use tsrs_lsproto as lsproto;
use tsrs_vfs::{FS, vfstest};

use crate::{
    APICreateProgramRequest, APIReconfigureProgramRequest, APISnapshotRequest, FileChangeSummary,
    SessionInit, SessionOptions, Snapshot, SyntheticProjectID, new_snapshot_host,
};

struct SnapshotLease(Arc<Snapshot>);

impl Drop for SnapshotLease {
    fn drop(&mut self) {
        self.0.deref();
    }
}

#[test]
fn synthetic_project_retains_configuration_and_reuses_unchanged_files() {
    let context = Context::background();
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(
        [("/main.ts", "export const value = 1;"), ("/other.ts", "export const other = 2;")],
        true,
    ));
    let init = SessionInit {
        background_ctx: context.clone(),
        options: Arc::new(SessionOptions { current_directory: "/".into(), ..Default::default() }),
        fs: Arc::clone(&fs),
        client: None,
        logger: None,
        npm_executor: None,
        parse_cache: None,
        content_mapped_parse_cache: None,
    };
    let host = new_snapshot_host(&init);
    let root = SnapshotLease(host.new_root_snapshot());
    let (snapshot, options_address) = {
        let region = Region::new(64 << 10);
        let _scope = region.enter();
        let options = P::new(CompilerOptions {
            module: ModuleKind::ESNext,
            no_lib: Tristate::True,
            strict: Tristate::True,
            target: ScriptTarget::ESNext,
            types: Some(Vec::new()),
            ..Default::default()
        });
        let request = APISnapshotRequest {
            create_programs: vec![APICreateProgramRequest {
                root_file_names: vec!["/main.ts".into(), "/other.ts".into()],
                compiler_options: Some(options),
                project_references: Vec::new(),
                config_file_parsing_diagnostics: Vec::new(),
                module_resolver_factory: None,
                module_resolver_id: 0,
            }],
            ensure_all_programs: true,
            ..Default::default()
        };
        let snapshot = host
            .clone_snapshot(
                &context,
                &root.0,
                FileChangeSummary::default(),
                Some(Arc::new(request)),
            )
            .unwrap_or_else(|(_, error)| panic!("{error:?}"));
        (SnapshotLease(snapshot), options.addr())
    };
    assert!(Region::containing(options_address).is_some());
    let id = SyntheticProjectID("/dev/null/synthetic/1".into());
    let program =
        snapshot.0.project_collection.get_project(&id.as_id()).unwrap().get_program().unwrap();
    let other = program.get_source_file("/other.ts").unwrap();
    let unchanged = SnapshotLease(
        host.clone_snapshot(
            &context,
            &snapshot.0,
            FileChangeSummary::default(),
            Some(Arc::new(APISnapshotRequest { ensure_all_programs: true, ..Default::default() })),
        )
        .unwrap_or_else(|(_, error)| panic!("{error:?}")),
    );
    assert!(std::ptr::eq(
        program,
        unchanged.0.project_collection.get_project(&id.as_id()).unwrap().get_program().unwrap()
    ));
    fs.write_file("/main.ts", "export const value = 'changed';").unwrap();
    let mut changes = FileChangeSummary::default();
    changes.changed.add(lsproto::DocumentUri("file:///main.ts".into()));
    let updated = SnapshotLease(
        host.clone_snapshot(
            &context,
            &unchanged.0,
            changes,
            Some(Arc::new(APISnapshotRequest { ensure_all_programs: true, ..Default::default() })),
        )
        .unwrap_or_else(|(_, error)| panic!("{error:?}")),
    );
    let updated_program =
        updated.0.project_collection.get_project(&id.as_id()).unwrap().get_program().unwrap();
    assert!(!std::ptr::eq(program, updated_program));
    assert_eq!(other, updated_program.get_source_file("/other.ts").unwrap());
    assert!(updated_program.get_source_file("/main.ts").unwrap().text().contains("changed"));
    let (reconfigured, configuration_address) = {
        let region = Region::new(64 << 10);
        let _scope = region.enter();
        let request = APISnapshotRequest {
            reconfigure_programs: vec![APIReconfigureProgramRequest {
                program_id: id.clone(),
                request: APICreateProgramRequest {
                    root_file_names: vec!["/main.ts".into()],
                    compiler_options: None,
                    project_references: Vec::new(),
                    config_file_parsing_diagnostics: Vec::new(),
                    module_resolver_factory: None,
                    module_resolver_id: 0,
                },
            }],
            ensure_all_programs: true,
            ..Default::default()
        };
        let snapshot = host
            .clone_snapshot(
                &context,
                &updated.0,
                FileChangeSummary::default(),
                Some(Arc::new(request)),
            )
            .unwrap_or_else(|(_, error)| panic!("{error:?}"));
        let program =
            snapshot.project_collection.get_project(&id.as_id()).unwrap().get_program().unwrap();
        (SnapshotLease(snapshot), program.command_line().addr())
    };
    drop(updated);
    drop(unchanged);
    drop(snapshot);
    let program =
        reconfigured.0.project_collection.get_project(&id.as_id()).unwrap().get_program().unwrap();
    assert!(program.options().strict.is_true());
    assert_eq!(program.command_line().file_names(), ["/main.ts"]);
    assert!(Region::containing(options_address).is_some());
    assert!(Region::containing(configuration_address).is_some());
    drop(reconfigured);
    assert!(Region::containing(options_address).is_none());
    assert!(Region::containing(configuration_address).is_none());
}
