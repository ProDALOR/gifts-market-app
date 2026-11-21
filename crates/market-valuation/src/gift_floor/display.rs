use crate::{DisplayDefinition, EvidenceDefinition as E, FieldDefinition as F};
pub(super) const DISPLAY: DisplayDefinition = DisplayDefinition {
    fields: &[
        F::currency("final_mark", "Final mark", Some(2)).primary(),
        F::currency("collection_floor", "Collection floor", None),
        F::currency("model_floor", "Model floor", None),
        F::currency("backdrop_floor", "Backdrop floor", None),
        F::currency("model_backdrop_floor", "Model & backdrop floor", None),
        F::number("model_multiplier", "Model multiplier", "×", Some(2)),
        F::number("backdrop_multiplier", "Backdrop multiplier", "×", Some(2)),
    ],
    evidence: &[
        E {
            path: "/evidence/collection/gifts",
            label: "distinct gifts",
        },
        E {
            path: "/evidence/collection/listings",
            label: "collection listings",
        },
    ],
    note: "Floor-based heuristic from current asks, before fees.",
};
