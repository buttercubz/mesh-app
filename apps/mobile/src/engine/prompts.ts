// Spoken-prompt templates for `setPrompts` (DESIGN §7.7): the `prompts` catalog in the
// rider's language, one template per mesh-api PromptKey. Sent on every startEngine and
// again whenever the language or the spoken-prompts setting changes while running.
import type { Lang, PromptKey, UiCommand } from "@meshapp/engine-api";
import { PROMPT_KEYS } from "@meshapp/engine-api";

import type { AppLanguage } from "../i18n/index";
import { catalogTexts } from "../i18n/index";

export type SetPromptsCommand = Extract<UiCommand, { type: "setPrompts" }>;

/** The engine's prompt language for an app language (both are es/en today). */
export function promptLang(language: AppLanguage): Lang {
  return language === "es" ? "es" : "en";
}

/** Every PromptKey's template from the registered `prompts` catalog (English fallback). */
export function promptTemplates(language: AppLanguage): Record<PromptKey, string> {
  const texts = catalogTexts("prompts", language);
  const english = catalogTexts("prompts", "en");
  const out = {} as Record<PromptKey, string>;
  for (const key of PROMPT_KEYS) {
    const text = texts[key] ?? english[key];
    if (text === undefined || text === "") throw new Error(`prompts catalog has no template for "${key}"`);
    out[key] = text;
  }
  return out;
}

/** The `setPrompts` command for the current settings. */
export function setPromptsCommand(language: AppLanguage, enabled: boolean): SetPromptsCommand {
  return { type: "setPrompts", lang: promptLang(language), enabled, templates: promptTemplates(language) };
}
