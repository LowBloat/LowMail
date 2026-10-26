import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { en } from "./en";
import { pt } from "./pt";

function files(dir: string): string[] {
	return readdirSync(dir).flatMap((name) => {
		const path = join(dir, name);
		if (statSync(path).isDirectory()) return files(path);
		return /\.(svelte|ts)$/.test(name) && !name.endsWith(".test.ts") && !path.includes("i18n") ? [path] : [];
	});
}

const source = files(join(import.meta.dir, "..", "..")).map((f) => readFileSync(f, "utf8")).join("\n");
const used = new Set([...source.matchAll(/\bt\("([a-zA-Z0-9_.]+)"/g)].map((m) => m[1]));
// Keys built at runtime.
for (const role of ["inbox", "sent", "drafts", "trash", "junk", "archive"]) used.add(`folder.${role}`);
for (const p of ["high", "normal", "low"]) used.add(`priority.${p}`);
for (const f of ["from", "to", "cc", "recipient", "subject", "body", "header", "size"]) used.add(`filters.field.${f}`);
for (const o of ["contains", "notContains", "is", "isNot", "startsWith", "endsWith", "over", "under"]) used.add(`filters.op.${o}`);
for (const a of ["move", "copy", "markRead", "flag", "forward", "discard", "reject"]) used.add(`filters.action.${a}`);
for (const k of ["auth", "network", "tls", "notFound", "invalid", "server", "unsupported", "tooLarge", "sessionExpired", "internal"]) used.add(`error.${k}`);
for (const m of source.matchAll(/label: "(shortcuts\.[a-zA-Z]+)"/g)) used.add(m[1]);

function has(dict: Record<string, string>, key: string) {
	return key in dict || (`${key}.one` in dict && `${key}.other` in dict);
}

describe("translations", () => {
	test("every key used exists in Portuguese and English", () => {
		const missingPt = [...used].filter((k) => !has(pt, k));
		const missingEn = [...used].filter((k) => !has(en, k));
		expect(missingPt).toEqual([]);
		expect(missingEn).toEqual([]);
	});

	test("both languages have the same keys and placeholders", () => {
		expect(Object.keys(en).sort()).toEqual(Object.keys(pt).sort());
		for (const key of Object.keys(pt)) {
			const vars = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
			expect(vars(en[key] ?? "")).toEqual(vars(pt[key]));
		}
	});
});
