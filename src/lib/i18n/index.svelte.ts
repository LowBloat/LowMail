/**
 * Interface text in Portuguese (Brazil) and English. `t("key", { n: 2 })`
 * reads the current language reactively; `{name}` placeholders are filled
 * from the second argument. Plural forms use `key.one` / `key.other`.
 */
import { setLocale } from "@codeatlasdev/ui";
import { en } from "./en";
import { pt } from "./pt";

export type Lang = "pt-BR" | "en";
export type Dict = Record<string, string>;

const dictionaries: Record<Lang, Dict> = { "pt-BR": pt, en };

function detect(): Lang {
	const langs = typeof navigator !== "undefined" ? navigator.languages ?? [navigator.language] : [];
	for (const l of langs) {
		if (l?.toLowerCase().startsWith("pt")) return "pt-BR";
		if (l?.toLowerCase().startsWith("en")) return "en";
	}
	return "pt-BR";
}

class I18n {
	lang = $state<Lang>(detect());

	set(choice: "auto" | Lang) {
		this.lang = choice === "auto" ? detect() : choice;
		try {
			setLocale(this.lang === "pt-BR" ? "pt-BR" : "en");
		} catch {
			// The component library may not know the language; it keeps its own.
		}
		if (typeof document !== "undefined") document.documentElement.lang = this.lang;
	}

	get dateLocale() {
		return this.lang === "pt-BR" ? "pt-BR" : "en-US";
	}
}

export const i18n = new I18n();

export function t(key: string, vars?: Record<string, string | number>): string {
	const dict = dictionaries[i18n.lang];
	let text: string | undefined;
	if (vars && typeof vars.n === "number") {
		const form = vars.n === 1 ? "one" : "other";
		text = dict[`${key}.${form}`] ?? pt[`${key}.${form}`];
	}
	text ??= dict[key] ?? pt[key] ?? key;
	if (vars) {
		text = text.replace(/\{(\w+)\}/g, (m, name) => (name in vars ? String(vars[name]) : m));
	}
	return text;
}

/** Translates an engine error kind, keeping the server's detail aside. */
export function errorText(e: unknown): string {
	if (e && typeof e === "object" && "kind" in e) {
		const kind = (e as { kind: string }).kind;
		const key = `error.${kind}`;
		const translated = t(key);
		const message = (e as { message?: string }).message ?? "";
		if (kind === "invalid" || kind === "notFound" || kind === "tooLarge") {
			// Validation messages from the engine are specific; show them.
			return i18n.lang === "en" || !message ? message || translated : translated + (message ? ` (${message})` : "");
		}
		return translated !== key ? translated : message;
	}
	return e instanceof Error ? e.message : String(e);
}

export function errorDetail(e: unknown): string | undefined {
	return e && typeof e === "object" && "detail" in e ? ((e as { detail?: string }).detail ?? undefined) : undefined;
}
