#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::*;
use context_engine::candidates::{Candidate, LineSpan};
use context_engine::compress::{default_level, render, Hints, Rendered};
use context_engine::model::{CandidateSource, CompressionLevel, ItemContent, ItemKind};
use review_core::location::DiffSide;

fn candidate(kind: ItemKind, side: DiffSide, start: u32, end: u32) -> Candidate {
    let mut c = Candidate::node(
        key(AUTHORIZE),
        AUTHORIZE,
        kind,
        side,
        CandidateSource::Structural,
    );
    c.span = Some(LineSpan {
        file: path("src/auth/auth.service.ts"),
        start_line: start,
        end_line: end,
        side,
    });
    c
}

fn segments(r: &Rendered) -> &Vec<context_engine::model::Segment> {
    match r {
        Rendered::Content {
            content: ItemContent::Code { segments, .. },
            ..
        } => segments,
        other => panic!("not code content: {other:?}"),
    }
}

const FILE: &str = "src/auth/auth.service.ts";

#[test]
fn auth_bypass_authorize_full_both_sides() {
    let blobs =
        MapBlobs::default()
            .with(&head(), FILE, &numbered(60))
            .with(&base(), FILE, &numbered(60));
    for (side, commit, end) in [(DiffSide::Head, head(), 30), (DiffSide::Base, base(), 34)] {
        let c = candidate(ItemKind::ChangedSymbol, side, 10, end);
        let r = render(
            &c,
            CompressionLevel::Full,
            &commit,
            &blobs,
            &Hints::default(),
        );
        let segs = segments(&r);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].location.side, side);
        assert_eq!(segs[0].location.commit, commit);
        assert_eq!(segs[0].location.start_line, 10);
        assert_eq!(segs[0].location.end_line, end);
    }
}

#[test]
fn large_method_changed_hunks_with_elision_markers() {
    let blobs = MapBlobs::default().with(&head(), FILE, &numbered(200));
    let c = candidate(ItemKind::ChangedSymbol, DiffSide::Head, 1, 150);
    let hints = Hints {
        changed_lines: &[60, 120],
        seed_call_lines: &[],
    };
    let r = render(&c, CompressionLevel::ChangedHunks, &head(), &blobs, &hints);
    let segs = segments(&r);
    assert_eq!(segs.len(), 3, "signature plus two hunks: {segs:?}");
    assert!(segs[1].elided_before.is_some());
    assert_eq!(segs[1].location.start_line, 57);
    assert_eq!(segs[1].location.end_line, 63);
    assert!(segs[2].elided_before.is_some());
    let Rendered::Content { content, .. } = &r else {
        panic!("no content")
    };
    assert!(content.text().contains("elided"));
}

#[test]
fn body_trimmed_keeps_seed_call_lines() {
    let blobs = MapBlobs::default().with(&head(), FILE, &numbered(100));
    let c = candidate(ItemKind::RelatedSymbol, DiffSide::Head, 1, 100);
    let hints = Hints {
        changed_lines: &[],
        seed_call_lines: &[50],
    };
    let r = render(&c, CompressionLevel::BodyTrimmed, &head(), &blobs, &hints);
    let Rendered::Content { content, .. } = &r else {
        panic!("no content")
    };
    let text = content.text();
    for kept in [
        "line 1\n",
        "line 15\n",
        "line 50\n",
        "line 51\n",
        "line 52\n",
        "line 96\n",
    ] {
        assert!(text.contains(kept), "missing {kept:?} in {text}");
    }
    for dropped in ["line 20\n", "line 49\n", "line 53\n", "line 95\n"] {
        assert!(!text.contains(dropped), "unexpected {dropped:?}");
    }
}

#[test]
fn class_signature_only_members() {
    let class = "export class AuthService {\n  constructor(private p: P) {}\n  authorize(user: U): boolean {\n    return this.p.check(user);\n  }\n  private helper(x: number) {\n    return x;\n  }\n}\n";
    let blobs = MapBlobs::default().with(&head(), FILE, class);
    let c = candidate(ItemKind::ClassSignature, DiffSide::Head, 1, 9);
    let r = render(
        &c,
        CompressionLevel::SignatureOnly,
        &head(),
        &blobs,
        &Hints::default(),
    );
    let Rendered::Content { content, .. } = &r else {
        panic!("no content")
    };
    let text = content.text();
    assert!(text.contains("export class AuthService"));
    assert!(text.contains("authorize(user: U)"));
    assert!(text.contains("private helper(x: number)"));
    assert!(!text.contains("return this.p.check"));
}

#[test]
fn locations_match_blob_lines() {
    let blob = numbered(200);
    let blobs = MapBlobs::default().with(&head(), FILE, &blob);
    let lines: Vec<&str> = blob.lines().collect();
    let c = candidate(ItemKind::ChangedSymbol, DiffSide::Head, 5, 150);
    let hints = Hints {
        changed_lines: &[40, 100],
        seed_call_lines: &[70],
    };
    for level in [
        CompressionLevel::Full,
        CompressionLevel::ChangedHunks,
        CompressionLevel::BodyTrimmed,
        CompressionLevel::SignatureOnly,
    ] {
        let r = render(&c, level, &head(), &blobs, &hints);
        for seg in segments(&r) {
            let from = usize::try_from(seg.location.start_line).unwrap() - 1;
            let to = usize::try_from(seg.location.end_line).unwrap();
            assert_eq!(
                seg.text,
                lines[from..to].join("\n"),
                "{level:?} {:?}",
                seg.location
            );
        }
    }
}

#[test]
fn secret_bearing_item_dropped() {
    let blob =
        "function connect() {\n  const password = 'hunter2hunter2';\n  return open(password);\n}\n";
    let blobs = MapBlobs::default().with(&head(), FILE, blob);
    let c = candidate(ItemKind::RelatedSymbol, DiffSide::Head, 1, 4);
    let r = render(
        &c,
        CompressionLevel::Full,
        &head(),
        &blobs,
        &Hints::default(),
    );
    assert_eq!(r, Rendered::SecretBearing);
}

#[test]
fn blob_failure_downgrades_to_reference() {
    let blobs = MapBlobs::default();
    let c = candidate(ItemKind::RelatedSymbol, DiffSide::Head, 1, 4);
    let r = render(
        &c,
        CompressionLevel::Full,
        &head(),
        &blobs,
        &Hints::default(),
    );
    let Rendered::Content {
        content,
        locations,
        downgraded,
    } = r
    else {
        panic!("expected content")
    };
    assert!(downgraded);
    assert_eq!(locations.len(), 1);
    assert!(matches!(
        content,
        ItemContent::Code {
            level: CompressionLevel::Reference,
            ..
        }
    ));
}

#[test]
fn default_levels_follow_the_spec() {
    assert_eq!(
        default_level(ItemKind::ChangedSymbol, Some(0), 1.0, 81),
        CompressionLevel::ChangedHunks
    );
    assert_eq!(
        default_level(ItemKind::ChangedSymbol, Some(0), 1.0, 80),
        CompressionLevel::Full
    );
    assert_eq!(
        default_level(ItemKind::RelatedSymbol, Some(1), 0.3, 41),
        CompressionLevel::BodyTrimmed
    );
    assert_eq!(
        default_level(ItemKind::RelatedSymbol, Some(1), 0.3, 40),
        CompressionLevel::Full
    );
    assert_eq!(
        default_level(ItemKind::RelatedSymbol, Some(2), 0.3, 10),
        CompressionLevel::SignatureOnly
    );
    assert_eq!(
        default_level(ItemKind::RelatedSymbol, Some(2), 0.6, 10),
        CompressionLevel::BodyTrimmed
    );
    assert_eq!(
        default_level(ItemKind::Test, Some(1), 0.5, 61),
        CompressionLevel::BodyTrimmed
    );
}
