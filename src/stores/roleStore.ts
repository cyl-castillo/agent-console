import { create } from "zustand";

import { jqlForRole, type ProjectRole, type SprintScope } from "../lib/jira";

/// Per-project memory of who is driving the console (developer/qa/analyst/
/// po/pm), per (project, role) an optional hand-tuned JQL that overrides the
/// role's preset, and per project the sprint scope checkboxes. Same
/// localStorage-mirror pattern as modelStore.

const ROLE_PREFIX = "agent-console:role:";
const JQL_PREFIX = "agent-console:jira-jql:";
const SPRINT_PREFIX = "agent-console:jira-sprint:";

const ROLES: ProjectRole[] = ["developer", "qa", "analyst", "po", "pm"];
const SPRINT_SCOPES: SprintScope[] = ["all", "active", "backlog", "active+backlog"];

function readRole(root: string): ProjectRole {
  try {
    const v = localStorage.getItem(ROLE_PREFIX + root);
    return ROLES.includes(v as ProjectRole) ? (v as ProjectRole) : "developer";
  } catch {
    return "developer";
  }
}

function readSprintScope(root: string): SprintScope {
  try {
    const v = localStorage.getItem(SPRINT_PREFIX + root);
    return SPRINT_SCOPES.includes(v as SprintScope) ? (v as SprintScope) : "all";
  } catch {
    return "all";
  }
}

function readJql(root: string, role: ProjectRole): string | null {
  try {
    const v = localStorage.getItem(`${JQL_PREFIX}${role}:${root}`);
    return v && v.trim() ? v : null;
  } catch {
    return null;
  }
}

interface RoleState {
  /// root -> role cache mirroring localStorage.
  roles: Record<string, ProjectRole | undefined>;
  /// (role|root) -> custom JQL cache; "" sentinel means explicitly cleared.
  jqls: Record<string, string | undefined>;
  /// root -> sprint scope cache mirroring localStorage.
  sprintScopes: Record<string, SprintScope | undefined>;

  roleFor: (root: string) => ProjectRole;
  setRoleFor: (root: string, role: ProjectRole) => void;
  sprintScopeFor: (root: string) => SprintScope;
  setSprintScopeFor: (root: string, scope: SprintScope) => void;
  /// Effective JQL for (project, role): the hand-tuned one if saved, else the
  /// role's preset.
  jqlFor: (root: string, role: ProjectRole) => string;
  /// True when the effective JQL differs from the role preset.
  hasCustomJql: (root: string, role: ProjectRole) => boolean;
  setJqlFor: (root: string, role: ProjectRole, jql: string | null) => void;
}

export const useRoleStore = create<RoleState>((set, get) => ({
  roles: {},
  jqls: {},
  sprintScopes: {},

  roleFor: (root) => get().roles[root] ?? readRole(root),

  sprintScopeFor: (root) => get().sprintScopes[root] ?? readSprintScope(root),

  setSprintScopeFor: (root, scope) => {
    try {
      if (scope === "all") localStorage.removeItem(SPRINT_PREFIX + root);
      else localStorage.setItem(SPRINT_PREFIX + root, scope);
    } catch {
      /* ignore */
    }
    set((s) => ({ sprintScopes: { ...s.sprintScopes, [root]: scope } }));
  },

  setRoleFor: (root, role) => {
    try {
      localStorage.setItem(ROLE_PREFIX + root, role);
    } catch {
      /* ignore */
    }
    set((s) => ({ roles: { ...s.roles, [root]: role } }));
  },

  jqlFor: (root, role) => {
    const k = `${role}|${root}`;
    const cached = get().jqls[k];
    const custom = cached !== undefined ? cached || null : readJql(root, role);
    return custom ?? jqlForRole(role);
  },

  hasCustomJql: (root, role) => get().jqlFor(root, role) !== jqlForRole(role),

  setJqlFor: (root, role, jql) => {
    const storageKey = `${JQL_PREFIX}${role}:${root}`;
    const trimmed = jql?.trim() ?? "";
    // Saving the preset itself (or clearing) removes the override.
    const effective = trimmed && trimmed !== jqlForRole(role) ? trimmed : null;
    try {
      if (effective) localStorage.setItem(storageKey, effective);
      else localStorage.removeItem(storageKey);
    } catch {
      /* ignore */
    }
    set((s) => ({ jqls: { ...s.jqls, [`${role}|${root}`]: effective ?? "" } }));
  },
}));
