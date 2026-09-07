/** Explicit bilingual messages. Never translate user data, protocol values or HTML. */
export type Locale = "zh-CN" | "en";
export declare const LANGUAGE_STORAGE_KEY = "sarmg.admin.language";
export declare function resolveLocale(): Locale;
export declare function getLocale(): Locale;
export declare function t(zh: string, en: string, values?: readonly (string | number)[]): string;
export declare function initializeLanguage(): void;
/** Reload deliberately: module-level messages and native clients share one locale.
 * Never persist forms or credentials, and never interrupt a pending form action.
 */
export declare function switchLanguage(): void;
export declare function languageLabel(): string;
export declare function validationMessage(input: HTMLInputElement | HTMLSelectElement): string;
export declare function createLanguageControl(): HTMLButtonElement;
