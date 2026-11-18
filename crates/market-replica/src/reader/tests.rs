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
    let (first, next) = scan(&view, &filter, Boundary::Ranked(None), 200, &|_| Ok(true)).unwrap();
    assert!(first.is_empty());
    let (second, next) = scan(&view, &filter, next.unwrap(), 200, &|_| Ok(true)).unwrap();
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
        let (first, next) = scan(&view, &filter, boundary, 200, &|_| Ok(true)).unwrap();
        assert_eq!(first.len(), 1);
        let (second, next) = scan(&view, &filter, next.unwrap(), 200, &|_| Ok(true)).unwrap();
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
    let error = scan(&view, &filter, Boundary::Price(None), 200, &|_| Ok(true))
        .err()
        .unwrap();
    assert_eq!(error.code(), tonic::Code::ResourceExhausted);
}

#[test]
fn row_policy_advances_filtered_boundaries_and_precedes_byte_accounting() {
    let dir = tempfile::tempdir().unwrap();
    let writer = ReplicaWriter::open(dir.path(), "test").unwrap();
    let mut state = State::default();
    state.apply(&catalog()).unwrap();
    for i in 0..10_002 {
        let mut row = order(&format!("order{i:05}"), "1", None);
        if i == 0 {
            row.meta.insert("large".into(), "x".repeat(513 * 1024));
        }
        state.upsert(entity(E::Order(row))).unwrap();
    }
    let view = view(state, &writer);
    let mut request = read(1, false);
    request.sort = 0;
    let filter = Filter::compile(&view.state, &request).unwrap();
    let include =
        |entity: &w::Entity| Ok(state::order(entity).unwrap().id.as_str() >= "order10000");
    let (first, next) = scan(&view, &filter, Boundary::Current(None), 1, &include).unwrap();
    assert!(first.is_empty());
    let (second, next) = scan(&view, &filter, next.unwrap(), 1, &include).unwrap();
    assert_eq!(state::order(&second[0]).unwrap().id, "order10000");
    let (third, next) = scan(&view, &filter, next.unwrap(), 1, &include).unwrap();
    assert_eq!(state::order(&third[0]).unwrap().id, "order10001");
    assert!(next.is_none());
    let error = scan(&view, &filter, Boundary::Current(None), 1, &|_| {
        Err(Status::cancelled("cancelled"))
    })
    .err()
    .unwrap();
    assert_eq!(error.code(), tonic::Code::Cancelled);
}

#[test]
fn checked_row_policy_fills_current_and_history_pages_for_every_rank() {
    use crate::test_support::upsert;
    let dir = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(dir.path(), "test").unwrap();
    let mut changes = catalog();
    for history in [false, true] {
        for i in 0..7 {
            let id = format!("{}-{i}", if history { "execution" } else { "ask" });
            let mut value = order(
                &id,
                &(i / 3 + 1).to_string(),
                history.then_some((i / 3 + 1) as i64),
            );
            value.observed_at_ms = (i / 3 + 1) as i64;
            changes.push(upsert(entity(E::Order(value))));
        }
    }
    let mut snapshot = writer.begin_snapshot().unwrap();
    snapshot.append(&changes).unwrap();
    writer
        .install_snapshot(snapshot.finish("policy").unwrap())
        .unwrap();
    let reader = writer.reader();
    for history in [false, true] {
        for mode in [w::ReadMode::Snapshot, w::ReadMode::Live] {
            for sort in 0..=6 {
                let mut request = read(mode as i32, history);
                request.sort = sort;
                request.limit = 2;
                let include = |entity: &w::Entity| {
                    Ok(state::order(entity).unwrap().id.as_bytes().last().unwrap() % 2 == 1)
                };
                let first = reader
                    .read_checked(request.clone(), |_, _| Ok(()), include)
                    .unwrap();
                assert_eq!(first.page.entities.len(), 2, "{history} {mode:?} {sort}");
                assert!(!first.page.next_page_token.is_empty());
                request.page_token = first.page.next_page_token;
                let second = reader
                    .read_checked(request, |_, _| Ok(()), include)
                    .unwrap();
                assert_eq!(second.page.entities.len(), 1);
                assert!(second.page.next_page_token.is_empty());
                let mut ids: Vec<_> = first
                    .page
                    .entities
                    .iter()
                    .chain(&second.page.entities)
                    .map(|e| state::order(e).unwrap().id.clone())
                    .collect();
                ids.sort();
                ids.dedup();
                assert_eq!(ids.len(), 3);
            }
        }
    }
}
