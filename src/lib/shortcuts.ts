/**
 * Keyboard shortcuts of the mail view (Gmail and Roundcube habits). Ignored
 * while typing and while a dialog is open.
 */
import { isDesktop } from "./api";
import { app } from "./state/app.svelte";
import { compose } from "./state/compose.svelte";
import { mail, parseKey } from "./state/mail.svelte";

export interface ShortcutHelp {
	keys: string[];
	label: string;
}

/** Shown in the help dialog (labels are i18n keys). */
export const SHORTCUTS: ShortcutHelp[] = [
	{ keys: ["C"], label: "shortcuts.compose" },
	{ keys: ["R"], label: "shortcuts.reply" },
	{ keys: ["A"], label: "shortcuts.replyAll" },
	{ keys: ["F"], label: "shortcuts.forward" },
	{ keys: ["J", "↓"], label: "shortcuts.next" },
	{ keys: ["K", "↑"], label: "shortcuts.previous" },
	{ keys: ["Enter"], label: "shortcuts.open" },
	{ keys: ["Esc"], label: "shortcuts.close" },
	{ keys: ["Del", "#"], label: "shortcuts.delete" },
	{ keys: ["Shift+Del"], label: "shortcuts.deleteForever" },
	{ keys: ["E"], label: "shortcuts.archive" },
	{ keys: ["!"], label: "shortcuts.junk" },
	{ keys: ["V"], label: "shortcuts.move" },
	{ keys: ["U"], label: "shortcuts.toggleRead" },
	{ keys: ["S", "*"], label: "shortcuts.flag" },
	{ keys: ["/"], label: "shortcuts.search" },
	{ keys: ["Ctrl+A"], label: "shortcuts.selectAll" },
	{ keys: ["F5"], label: "shortcuts.refresh" },
	{ keys: ["Ctrl+Enter"], label: "shortcuts.send" },
	{ keys: ["?"], label: "shortcuts.help" },
];

function typing(target: EventTarget | null) {
	const el = target as HTMLElement | null;
	return !!el?.closest?.("input, textarea, select, [contenteditable=''], [contenteditable='true'], iframe");
}

function modalOpen() {
	return !!document.querySelector('[role="dialog"][aria-modal="true"], [role="alertdialog"], dialog[open], [role="menu"]');
}

/** The message the reply/forward shortcuts act on. */
function source() {
	const key = mail.open ?? mail.focused;
	return key ? parseKey(key) : null;
}

export function handleShortcut(e: KeyboardEvent) {
	if (e.defaultPrevented || typing(e.target) || modalOpen()) return;
	if (app.view !== "mail" || compose.visible) return;
	const ctrl = e.ctrlKey || e.metaKey;
	const k = e.key;
	let handled = true;
	if (ctrl && k.toLowerCase() === "a") mail.selectAll();
	else if (ctrl || e.altKey) handled = false;
	else if (k === "c" || k === "n") void compose.open("new");
	else if (k === "r" && source()) void compose.open("reply", source());
	else if (k === "a" && source()) void compose.open("replyAll", source());
	else if (k === "f" && source()) void compose.open("forward", source());
	else if (k === "j" || k === "ArrowDown") mail.move(1, e.shiftKey);
	else if (k === "k" || k === "ArrowUp") mail.move(-1, e.shiftKey);
	else if ((k === "Enter" || k === "o") && mail.focused) void mail.openMessage(mail.focused);
	else if (k === "Escape") {
		if (mail.selected.size) mail.selectNone();
		else mail.closeMessage();
	} else if (k === "Delete" || k === "#") void mail.remove(undefined, e.shiftKey);
	else if (k === "e") void mail.archive();
	else if (k === "!") void mail.junk(true);
	else if (k === "v" || k === "m") window.dispatchEvent(new CustomEvent("lm:move"));
	else if (k === "u") void mail.toggleRead();
	else if (k === "U") void mail.setFlags(mail.targets, { seen: false });
	else if (k === "I") void mail.setFlags(mail.targets, { seen: true });
	else if (k === "s" || k === "*") void mail.toggleFlag();
	else if (k === "/") window.dispatchEvent(new CustomEvent("lm:search"));
	else if (k === "F5" && isDesktop) mail.refresh();
	else handled = false;
	if (handled) e.preventDefault();
}
