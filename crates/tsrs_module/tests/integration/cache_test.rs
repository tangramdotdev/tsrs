use tsrs_module::types::ModeAwareCacheKey;

use tsrs_core::ModuleKind;

use tsrs_module::cache::*;

#[test]
fn mode_aware_cache_get_borrows_name_and_checks_mode() {
    let mut cache = ModeAwareCache::default();
    cache.insert(ModeAwareCacheKey { attributes: None, name: "pkg", mode: ModuleKind::ES_NEXT }, 1);
    cache.insert(
        ModeAwareCacheKey { attributes: None, name: "pkg", mode: ModuleKind::COMMON_JS },
        2,
    );

    assert_eq!(mode_aware_cache_get(&cache, "pkg", ModuleKind::ES_NEXT), Some(&1));
    assert_eq!(mode_aware_cache_get(&cache, "pkg", ModuleKind::COMMON_JS), Some(&2));
    assert_eq!(mode_aware_cache_get(&cache, "other", ModuleKind::ES_NEXT), None);
}

#[test]
fn mode_aware_cache_get_with_attributes_partitions_entries() {
    let mut cache = ModeAwareCache::default();
    cache.insert(ModeAwareCacheKey { attributes: None, name: "pkg", mode: ModuleKind::ES_NEXT }, 1);
    cache.insert(
        ModeAwareCacheKey {
            attributes: Some("{ kind: \"number\" }"),
            name: "pkg",
            mode: ModuleKind::ES_NEXT,
        },
        2,
    );

    let attributes = String::from("{ kind: \"number\" }");
    assert_eq!(mode_aware_cache_get(&cache, "pkg", ModuleKind::ES_NEXT), Some(&1));
    assert_eq!(
        mode_aware_cache_get_with_attributes(&cache, "pkg", ModuleKind::ES_NEXT, Some(&attributes)),
        Some(&2)
    );
    assert_eq!(
        mode_aware_cache_get_with_attributes(&cache, "pkg", ModuleKind::ES_NEXT, Some("{}")),
        None
    );
}
