// Once-per-ride application of the rider's default talk mode (§1.4): the ride screen
// remounts when the rider opens Members and comes back, which must not undo a mode the
// rider picked in the meantime.
let appliedFor: string | null = null;

/** `true` the first time it is called for `groupId` (until reset or another group). */
export function claimDefaultMode(groupId: string): boolean {
  if (appliedFor === groupId) return false;
  appliedFor = groupId;
  return true;
}

/** Forgets the claim (leaving a ride, tests). */
export function resetRideDefaults(): void {
  appliedFor = null;
}
