export const teamRoles = ["owner", "admin", "member"] as const;
export type TeamRole = (typeof teamRoles)[number];

export function isTeamRole(value: string): value is TeamRole {
  return teamRoles.includes(value as TeamRole);
}

export function canManageMembers(role: TeamRole) {
  return role === "owner" || role === "admin";
}

export function canManageOwners(role: TeamRole) {
  return role === "owner";
}

export function canRemoveRole(actorRole: TeamRole, targetRole: TeamRole) {
  if (actorRole === "owner") {
    return targetRole !== "owner";
  }
  if (actorRole === "admin") {
    return targetRole === "member";
  }
  return false;
}

export function canAssignRole(actorRole: TeamRole, nextRole: TeamRole) {
  if (actorRole === "owner") {
    return true;
  }
  if (actorRole === "admin") {
    return nextRole !== "owner";
  }
  return false;
}
