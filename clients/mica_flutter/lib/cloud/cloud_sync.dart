// P2-M4 cloud sync session facade. One shared protocol state machine
// (cloud_sync_session.dart) runs on every platform; the CRDT engine behind it
// is chosen by the replica seam — Rust `yrs` over FFI on desktop, JS `yjs` on
// web (wire-compatible, so both converge on the same document). See
// sync_doc_replica.dart for the seam.
export 'cloud_sync_session.dart';
