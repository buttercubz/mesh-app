import type { IntercomPermission } from "../../engine/client";

/** The permissions asked in onboarding (camera when the scanner opens; location with sharing). */
export const ONBOARDING_PERMISSIONS: readonly IntercomPermission[] = ["microphone", "bluetooth", "notifications"];
