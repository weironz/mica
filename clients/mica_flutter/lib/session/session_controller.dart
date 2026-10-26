import 'package:flutter/foundation.dart';

import '../api/models.dart';

/// The shell's session/workspace context — the four "hub" fields every domain
/// of `_WorkspaceShellState` used to touch directly (dry-run: `_session` was
/// read by 11 semantic domains, `_selectedWorkspace` by 9, `_workspaces` by 6,
/// `_viewsByWorkspace` by 5). Extracted in the god-class third step — see
/// docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7. Collecting them here
/// is what makes the later domain extractions cheap: domains take this store
/// instead of reaching back into the State.
///
/// The State keeps its four legacy fields as synced view-model copies, so
/// build helpers read them unchanged; every mutation goes through a transition
/// method below.
class SessionController extends ChangeNotifier {
  AuthSession? session;
  List<Workspace> workspaces = const [];
  Workspace? selectedWorkspace;
  Map<String, List<DocumentView>> viewsByWorkspace = const {};

  /// Identity epoch for async requests. Full sign-in/restore and session end
  /// retire earlier requests even when callers reuse the same session/list
  /// objects; token rotation and profile refresh preserve this epoch.
  int get generation => _generation;
  int _generation = 0;

  /// A full sign-in or cold restore: everything lands at once, selection as
  /// the caller computed it (the saved id when it still exists, else the
  /// first workspace).
  void restore(
    AuthSession signedIn,
    List<Workspace> list,
    Workspace? selected,
    Map<String, List<DocumentView>> views,
  ) {
    _generation++;
    session = signedIn;
    workspaces = list;
    selectedWorkspace = selected;
    viewsByWorkspace = views;
    notifyListeners();
  }

  /// Tokens rotated or the profile refreshed: only the session object moves,
  /// the workspace state is untouched.
  void adoptSession(AuthSession updated) {
    session = updated;
    notifyListeners();
  }

  /// A fresh interactive sign-in: no saved selection to honor, so the first
  /// workspace is the landing spot.
  void signIn(AuthSession signedIn, List<Workspace> list) {
    _generation++;
    session = signedIn;
    workspaces = list;
    selectedWorkspace = list.firstOrNull;
    notifyListeners();
  }

  /// The sign-in is over and cannot be renewed. Only the SESSION drops — the
  /// workspace list deliberately survives so the local world and any UI still
  /// holding it can fall back coherently; full clearing is [signOut].
  void expireSession() {
    _generation++;
    session = null;
    notifyListeners();
  }

  /// Signing out: everything cloud-side goes.
  void signOut() {
    _generation++;
    session = null;
    workspaces = const [];
    selectedWorkspace = null;
    viewsByWorkspace = const {};
    notifyListeners();
  }

  /// The list arrived, with NO selection opinion of its own — the selection
  /// is left exactly as it was (even if its workspace vanished from this
  /// list; a later [replaceWorkspaces] reconciles). The plain refetch paths
  /// (import finished, world migrated) behaved this way before the store
  /// existed; [replaceWorkspaces] is the reconciling variant.
  void setWorkspaces(List<Workspace> list) {
    workspaces = list;
    notifyListeners();
  }

  /// The workspace list was re-fetched. Selection rule (pinned by tests): a
  /// selected workspace SURVIVES when its id is still in the list, and falls
  /// to NULL when it is not — not to the first workspace, because arriving
  /// where the user did not put themselves is a lie; an empty selection is at
  /// least honest. A null selection takes the first workspace (there was
  /// nothing to preserve).
  void replaceWorkspaces(List<Workspace> list) {
    workspaces = list;
    selectedWorkspace = selectedWorkspace == null
        ? list.firstOrNull
        : list
            .where((workspace) => workspace.id == selectedWorkspace!.id)
            .firstOrNull;
    notifyListeners();
  }

  void select(Workspace workspace) {
    selectedWorkspace = workspace;
    notifyListeners();
  }

  /// A workspace rename came back: replace it in place by id and keep it
  /// selected (renaming the workspace you are in must not move you).
  void applyRename(Workspace renamed) {
    workspaces = workspaces
        .map((item) => item.id == renamed.id ? renamed : item)
        .toList();
    selectedWorkspace = renamed;
    notifyListeners();
  }

  /// A newly created workspace: appended, selected, with its first page's
  /// tree seeded. Returns nothing — the caller also opens that page, which is
  /// tab state this store does not own.
  void addCreated(Workspace workspace, List<DocumentView> firstViews) {
    workspaces = [...workspaces, workspace];
    selectedWorkspace = workspace;
    viewsByWorkspace = {
      ...viewsByWorkspace,
      workspace.id: firstViews,
    };
    notifyListeners();
  }

  /// A workspace was deleted. Unlike [replaceWorkspaces], the selection falls
  /// to the FIRST remaining workspace when the deleted one was selected —
  /// deletion is an explicit user act on a known target, and leaving them on
  /// an empty selection after THEY pulled the trigger reads as a bug, not
  /// honesty. Returns whether the deleted workspace had been selected, so the
  /// caller can close the page it held (tab state, not ours).
  bool remove(Workspace workspace) {
    final wasSelected = selectedWorkspace?.id == workspace.id;
    final remaining =
        workspaces.where((item) => item.id != workspace.id).toList();
    workspaces = remaining;
    viewsByWorkspace = {...viewsByWorkspace}..remove(workspace.id);
    if (wasSelected) {
      selectedWorkspace = remaining.isNotEmpty ? remaining.first : null;
    }
    notifyListeners();
    return wasSelected;
  }

  /// One workspace's page tree was fetched or changed server-side.
  void replaceViews(String workspaceId, List<DocumentView> views) {
    viewsByWorkspace = {
      ...viewsByWorkspace,
      workspaceId: views,
    };
    notifyListeners();
  }
}
