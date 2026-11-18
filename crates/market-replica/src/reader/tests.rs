use super::*;
use crate::test_support::{catalog, entity, order, read};
use w::entity::Entity as E;

fn view(state: State, writer: &ReplicaWriter) -> ReplicaView {
    ReplicaView {
        revisions: crate::revisions::Revisions::replaced(None, &state),
        state,
        history: writer.store.snapshot().unwrap(),
        cursor: "test".into(),
    }
}

#[test]
fn borrowed_ranked_scan_can_resume_after_an_empty_scan_budget_page() {
    let dir = tempfile::tempdir().unwrap();
    let writer = ReplicaWriter::open(dir.path(), "test").unwrap();
    let mut state = State::default();
    state.apply(&catalog()).unwrap();
    for i in 0..10_002 {
        state
            .upsert(entity(E::Order(order(
                &format!("order{i:05}"),
                if i < 10_000 { "1" } else { "2" },
                None,
            ))))
            .unwrap();
    }
    let view = view(state, &writer);
    let mut request = read(1, false);
    request.sort = 3; // Time order cannot skip candidates using the price index.
    let Some(w::selection::Source::Scope(scope)) = &mut request.selection.as_mut().unwrap().source
    else {
        unreachable!()
    };
    scope.price = Some(w::PriceRange {
        min: Some("2".into()),
        max: None,
    });
    let filter = Filter::compile(&view.state, &request).unwrap();
    let (first, next) = scan(&view, &filter, Boundary::Ranked(None), 200).unwrap();
    assert!(first.is_empty());
    let (second, next) = scan(&view, &filter, next.unwrap(), 200).unwrap();
    assert_eq!(
        second
            .iter()
            .map(|e| state::order(e).unwrap().id.as_str())
            .collect::<Vec<_>>(),
        ["order10000", "order10001"]
    );
    assert!(next.is_none());
}

#[test]
fn borrowed_rows_preserve_page_byte_limits_and_oversized_record_errors() {
    let dir = tempfile::tempdir().unwrap();
    let writer = ReplicaWriter::open(dir.path(), "test").unwrap();
    let mut state = State::default();
    state.apply(&catalog()).unwrap();
    for id in ["a", "b"] {
        let mut value = order(id, "1", None);
        value.meta.insert("large".into(), "x".repeat(300 * 1024));
        state.upsert(entity(E::Order(value))).unwrap();
    }
    let mut view = view(state, &writer);
    for sort in 0..=6 {
        let mut request = read(1, false);
        request.sort = sort;
        let filter = Filter::compile(&view.state, &request).unwrap();
        let boundary = match sort {
            0 => Boundary::Current(None),
            1 | 2 => Boundary::Price(None),
            _ => Boundary::Ranked(None),
        };
        let (first, next) = scan(&view, &filter, boundary, 200).unwrap();
        assert_eq!(first.len(), 1);
        let (second, next) = scan(&view, &filter, next.unwrap(), 200).unwrap();
        assert_eq!(second.len(), 1);
        assert_ne!(
            state::order(&first[0]).unwrap().id,
            state::order(&second[0]).unwrap().id
        );
        assert!(next.is_none());
    }
    let mut huge = order("a", "1", None);
    huge.meta.insert("large".into(), "x".repeat(513 * 1024));
    view.state.upsert(entity(E::Order(huge))).unwrap();
    let filter = Filter::compile(&view.state, &read(1, false)).unwrap();
    let error = scan(&view, &filter, Boundary::Price(None), 200)
        .err()
        .unwrap();
    assert_eq!(error.code(), tonic::Code::ResourceExhausted);
}
