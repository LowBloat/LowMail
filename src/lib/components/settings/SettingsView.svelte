<script lang="ts">
	import { Button, FormSection, SegmentedControl, Select, SettingItem, Switch } from "@codeatlasdev/ui";
	import { BookOpen, Filter, Folder, Info, PenLine, Plane, Settings, UserRound, Users } from "@lucide/svelte";
	import { isDesktop } from "../../api";
	import { app } from "../../state/app.svelte";
	import { updates } from "../../state/updates.svelte";
	import { t } from "../../i18n/index.svelte";
	import { requestNotificationPermission } from "../../notify";
	import type { Settings as S } from "../../types";
	import IdentitiesSettings from "./IdentitiesSettings.svelte";
	import FoldersSettings from "./FoldersSettings.svelte";
	import FiltersSettings from "./FiltersSettings.svelte";
	import VacationSettings from "./VacationSettings.svelte";
	import AccountsSettings from "./AccountsSettings.svelte";

	type Section = "general" | "reading" | "composing" | "identities" | "folders" | "filters" | "vacation" | "accounts" | "about";
	let section = $state<Section>("general");

	const sections = $derived(
		[
			{ id: "general", label: t("settings.general"), icon: Settings },
			{ id: "reading", label: t("settings.reading"), icon: BookOpen },
			{ id: "composing", label: t("settings.composing"), icon: PenLine },
			{ id: "identities", label: t("settings.identities"), icon: UserRound },
			{ id: "folders", label: t("settings.folders"), icon: Folder },
			app.account?.filters && { id: "filters", label: t("settings.filters"), icon: Filter },
			app.account?.filters && { id: "vacation", label: t("settings.vacation"), icon: Plane },
			isDesktop && { id: "accounts", label: t("settings.accounts"), icon: Users },
			{ id: "about", label: t("settings.about"), icon: Info },
		].filter(Boolean) as { id: Section; label: string; icon: typeof Settings }[],
	);

	const s = $derived(app.settings);

	function set<K extends keyof S>(key: K, value: S[K]) {
		void app.saveSettings({ [key]: value } as Partial<S>);
	}

	async function toggleNotifications(on: boolean) {
		if (on && !(await requestNotificationPermission())) {
			set("notifications", false);
			return;
		}
		set("notifications", on);
	}

	const opts = (pairs: [string, string][]) => pairs.map(([value, label]) => ({ value, label }));
</script>

<div class="lm-settings">
	<nav class="lm-settings__nav" aria-label={t("nav.settings")}>
		<div class="lm-bar"><span class="lm-bar__title">{t("nav.settings")}</span></div>
		{#each sections as item (item.id)}
			<button type="button" class:active={section === item.id} aria-current={section === item.id ? "page" : undefined} onclick={() => (section = item.id)}>
				<item.icon size={16} />
				{item.label}
			</button>
		{/each}
	</nav>

	<div class="lm-settings__body lm-scroll">
		<div class="lm-settings__content">
			{#if section === "general"}
				<h2>{t("settings.general")}</h2>
				<FormSection title={t("settings.appearance")} layout="list">
					<SettingItem label={t("settings.language")}>
						<Select size="sm" value={s.language} options={opts([["auto", t("settings.language.auto")], ["pt-BR", "Português (Brasil)"], ["en", "English"]])} onchange={(v) => set("language", v as S["language"])} />
					</SettingItem>
					<SettingItem label={t("settings.theme")}>
						<SegmentedControl size="sm" value={s.theme} items={opts([["system", t("settings.theme.system")], ["light", t("settings.theme.light")], ["dark", t("settings.theme.dark")]])} onchange={(v) => set("theme", v as S["theme"])} />
					</SettingItem>
					<SettingItem label={t("settings.layout")} description={t("settings.layout.hint")}>
						<SegmentedControl size="sm" value={s.layout} items={opts([["columns", t("settings.layout.columns")], ["rows", t("settings.layout.rows")], ["list", t("settings.layout.list")]])} onchange={(v) => set("layout", v as S["layout"])} />
					</SettingItem>
					<SettingItem label={t("settings.density")}>
						<SegmentedControl size="sm" value={s.density} items={opts([["compact", t("settings.density.compact")], ["comfortable", t("settings.density.comfortable")]])} onchange={(v) => set("density", v as S["density"])} />
					</SettingItem>
				</FormSection>
				<FormSection title={t("settings.notifications")} layout="list">
					<SettingItem label={t("settings.notifications.new")} description={t("settings.notifications.hint")}>
						<Switch checked={s.notifications} aria-label={t("settings.notifications.new")} onchange={toggleNotifications} />
					</SettingItem>
				</FormSection>
			{:else if section === "reading"}
				<h2>{t("settings.reading")}</h2>
				<FormSection title={t("settings.reading.messages")} layout="list">
					<SettingItem label={t("settings.markRead")}>
						<Select
							size="sm"
							value={String(s.markReadDelay)}
							options={opts([["0", t("settings.markRead.now")], ["2", t("settings.markRead.after", { n: 2 })], ["5", t("settings.markRead.after", { n: 5 })], ["10", t("settings.markRead.after", { n: 10 })], ["-1", t("settings.markRead.never")]])}
							onchange={(v) => set("markReadDelay", Number(v))}
						/>
					</SettingItem>
					<SettingItem label={t("settings.images")} description={t("settings.images.hint")}>
						<Select size="sm" value={s.showImages} options={opts([["never", t("settings.images.never")], ["contacts", t("settings.images.trusted")], ["always", t("settings.images.always")]])} onchange={(v) => set("showImages", v as S["showImages"])} />
					</SettingItem>
					<SettingItem label={t("settings.receipts")}>
						<Select size="sm" value={s.readReceipts} options={opts([["ask", t("settings.receipts.ask")], ["always", t("settings.receipts.always")], ["never", t("settings.receipts.never")]])} onchange={(v) => set("readReceipts", v as S["readReceipts"])} />
					</SettingItem>
					<SettingItem label={t("settings.confirmDelete")}>
						<Switch checked={s.confirmPermanentDelete} aria-label={t("settings.confirmDelete")} onchange={(v) => set("confirmPermanentDelete", v)} />
					</SettingItem>
					<SettingItem label={t("settings.pageSize")}>
						<Select size="sm" value={String(s.pageSize)} options={opts([["50", "50"], ["100", "100"], ["200", "200"], ["500", "500"]])} onchange={(v) => set("pageSize", Number(v))} />
					</SettingItem>
				</FormSection>
				{#if s.trustedSenders.length}
					<FormSection title={t("settings.trusted")} description={t("settings.trusted.hint")} layout="list">
						{#each s.trustedSenders as sender (sender)}
							<SettingItem label={sender}>
								<Button variant="ghost" size="sm" onclick={() => set("trustedSenders", s.trustedSenders.filter((x) => x !== sender))}>{t("action.remove")}</Button>
							</SettingItem>
						{/each}
					</FormSection>
				{/if}
			{:else if section === "composing"}
				<h2>{t("settings.composing")}</h2>
				<FormSection title={t("settings.composing.messages")} layout="list">
					<SettingItem label={t("settings.html")} description={t("settings.html.hint")}>
						<Switch checked={s.composeHtml} aria-label={t("settings.html")} onchange={(v) => set("composeHtml", v)} />
					</SettingItem>
					<SettingItem label={t("settings.replyOnTop")} description={t("settings.replyOnTop.hint")}>
						<Switch checked={s.replyOnTop} aria-label={t("settings.replyOnTop")} onchange={(v) => set("replyOnTop", v)} />
					</SettingItem>
					<SettingItem label={t("settings.saveSent")} description={t("settings.saveSent.hint")}>
						<Switch checked={s.saveSent} aria-label={t("settings.saveSent")} onchange={(v) => set("saveSent", v)} />
					</SettingItem>
					<SettingItem label={t("settings.autosave")}>
						<Select size="sm" value={String(s.autosaveSeconds)} options={opts([["30", t("settings.autosave.every", { n: 30 })], ["60", t("settings.autosave.every", { n: 60 })], ["300", t("settings.autosave.every", { n: 300 })], ["0", t("settings.autosave.off")]])} onchange={(v) => set("autosaveSeconds", Number(v))} />
					</SettingItem>
				</FormSection>
			{:else if section === "identities"}
				<IdentitiesSettings />
			{:else if section === "folders"}
				<FoldersSettings />
			{:else if section === "filters"}
				<FiltersSettings />
			{:else if section === "vacation"}
				<VacationSettings />
			{:else if section === "accounts"}
				<AccountsSettings />
			{:else}
				<h2>{t("settings.about")}</h2>
				<div class="lm-about">
					<img src="./favicon.svg" alt="" width="56" height="56" />
					<div>
						<p class="lm-about__name">{t("about.name", { version: app.version })}</p>
						<p class="lm-muted">{t("about.text")}</p>
						<p class="lm-hint">{t("about.account", { email: app.account?.email ?? "" })}</p>
						{#if app.account?.capabilities.length}
							<details>
								<summary class="lm-hint">{t("about.capabilities")}</summary>
								<p class="lm-hint lm-caps">{app.account.capabilities.join(" ")}</p>
							</details>
						{/if}
						<p class="lm-hint">{t("about.license")}</p>
						{#if updates.supported}
							<div class="lm-about__update">
								{#if updates.available}
									<Button variant="filled" size="sm" onclick={() => (updates.dialogOpen = true)}>{t("updates.available", { version: updates.available.version })}</Button>
								{:else}
									<Button variant="outlined" size="sm" loading={updates.checking} onclick={() => updates.check(true)}>{t("updates.check")}</Button>
								{/if}
								{#if updates.upToDate}<span class="lm-hint" role="status">{t("updates.upToDate")}</span>{/if}
								{#if updates.error && !updates.dialogOpen}<span class="lm-hint lm-danger" role="alert">{updates.error}</span>{/if}
							</div>
						{/if}
					</div>
				</div>
			{/if}
		</div>
	</div>
</div>

<style>
	.lm-settings {
		flex: 1;
		min-width: 0;
		display: flex;
	}
	.lm-settings__nav {
		width: var(--lm-settings-nav-w);
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		background: var(--ca-surface-sunken);
		border-right: 1px solid var(--ca-outline-subtle);
	}
	.lm-settings__nav button {
		display: flex;
		align-items: center;
		gap: var(--ca-space-3-5);
		height: var(--lm-control-lg);
		padding: 0 var(--ca-space-4-5);
		border: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.lm-settings__nav button:hover {
		background: var(--lm-row-hover);
		color: var(--ca-text-primary);
	}
	.lm-settings__nav button.active {
		background: var(--ca-surface-base);
		color: var(--ca-text-primary);
		font-weight: 600;
		box-shadow: inset var(--lm-accent) 0 0 var(--lm-signal);
	}
	.lm-settings__body {
		flex: 1;
	}
	.lm-settings__content {
		max-width: var(--lm-settings-w);
		padding: var(--ca-space-7) var(--ca-space-10) var(--ca-space-12);
		display: flex;
		flex-direction: column;
		gap: var(--ca-space-7);
	}
	.lm-settings__content :global(h2) {
		margin: 0;
		font-size: var(--ca-font-size-xl);
	}
	.lm-about {
		display: flex;
		gap: var(--ca-space-6);
		align-items: flex-start;
	}
	.lm-about__update {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--ca-space-3);
		margin-top: var(--ca-space-3);
	}
	.lm-danger {
		color: var(--lm-danger);
	}
	.lm-about p {
		margin: 0 0 var(--ca-space-2);
	}
	.lm-about__name {
		font-size: var(--ca-font-size-h4);
		font-weight: 700;
	}
	.lm-caps {
		font-family: var(--ca-font-mono);
		max-width: var(--lm-caps-w);
	}
	@media (max-width: 760px) {
		.lm-settings {
			flex-direction: column;
		}
		.lm-settings__nav {
			width: auto;
			flex-direction: row;
			overflow-x: auto;
		}
		.lm-settings__nav .lm-bar {
			display: none;
		}
		.lm-settings__nav button {
			white-space: nowrap;
		}
	}
</style>
