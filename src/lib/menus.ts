/** The desktop application menus (title bar on Windows and Linux, menu bar on macOS). */
import { colorScheme } from "@codeatlasdev/ui";
import { IS_MAC, appMenu, type AppMenu } from "./app-menu";
import { app } from "./state/app.svelte";
import { compose } from "./state/compose.svelte";
import { mail, parseKey } from "./state/mail.svelte";
import { t } from "./i18n/index.svelte";

const SEPARATOR = { separator: true } as const;

export function buildMenus(actions: { shortcuts: () => void; about: () => void; quit: () => void; docs: () => void }): AppMenu[] {
	const open = mail.open ? parseKey(mail.open) : null;
	const hasTarget = mail.targets.length > 0;
	const signedIn = app.signedIn;
	return [
		appMenu("file", t("menu.file"), [
			{ label: t("action.compose"), shortcut: "Ctrl+N", disabled: !signedIn, action: () => compose.open("new") },
			{ label: t("account.add"), action: () => (app.editingAccount = null) },
			SEPARATOR,
			{ label: t("nav.contacts"), disabled: !signedIn, action: () => (app.view = "contacts") },
			{ label: t("nav.settings"), disabled: !signedIn, action: () => (app.view = "settings") },
			!IS_MAC && SEPARATOR,
			!IS_MAC && { label: t("menu.quit"), action: actions.quit },
		]),
		appMenu("message", t("menu.message"), [
			{ label: t("action.reply"), shortcut: "R", disabled: !open, action: () => open && compose.open("reply", open) },
			{ label: t("action.replyAll"), shortcut: "A", disabled: !open, action: () => open && compose.open("replyAll", open) },
			{ label: t("action.forward"), shortcut: "F", disabled: !open, action: () => open && compose.open("forward", open) },
			SEPARATOR,
			{ label: t("action.toggleRead"), shortcut: "U", disabled: !hasTarget, action: () => mail.toggleRead() },
			{ label: t("action.flag"), shortcut: "S", disabled: !hasTarget, action: () => mail.toggleFlag() },
			{ label: t("action.moveTo"), shortcut: "V", disabled: !hasTarget, action: () => window.dispatchEvent(new CustomEvent("lm:move")) },
			{ label: t("action.archive"), shortcut: "E", disabled: !hasTarget, action: () => mail.archive() },
			{ label: t("action.junk"), shortcut: "!", disabled: !hasTarget, action: () => mail.junk(true) },
			{ label: t("action.delete"), shortcut: "Del", disabled: !hasTarget, action: () => mail.remove() },
		]),
		appMenu("view", t("menu.view"), [
			{ label: t("settings.layout.columns"), checked: app.settings.layout === "columns", disabled: !signedIn, action: () => app.saveSettings({ layout: "columns" }) },
			{ label: t("settings.layout.rows"), checked: app.settings.layout === "rows", disabled: !signedIn, action: () => app.saveSettings({ layout: "rows" }) },
			{ label: t("settings.layout.list"), checked: app.settings.layout === "list", disabled: !signedIn, action: () => app.saveSettings({ layout: "list" }) },
			SEPARATOR,
			{ label: t("menu.darkTheme"), checked: colorScheme.isDark, action: () => app.saveSettings({ theme: colorScheme.isDark ? "light" : "dark" }) },
			{ label: t("action.refresh"), shortcut: "F5", disabled: !signedIn, action: () => mail.refresh() },
		]),
		appMenu("help", t("menu.help"), [
			{ label: t("shortcuts.title"), shortcut: "?", action: actions.shortcuts },
			{ label: t("menu.docs"), action: actions.docs },
			SEPARATOR,
			{ label: t("menu.about"), action: actions.about },
		]),
	];
}
