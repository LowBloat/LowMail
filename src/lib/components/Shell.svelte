<script lang="ts" module>
	/** Startup mailto: links are handled once, not again after switching accounts. */
	let startupHandled = false;
</script>

<script lang="ts">
	import { onDestroy } from "svelte";
	import { Avatar, DropdownMenu, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, Tooltip } from "@codeatlasdev/ui";
	import { ArrowUpCircle, Check, LogOut, Mail, Plus, Settings, SquarePen, UserRound, WifiOff } from "@lucide/svelte";
	import { isDesktop } from "../api";
	import { app, type View } from "../state/app.svelte";
	import { compose } from "../state/compose.svelte";
	import { updates } from "../state/updates.svelte";
	import { mail } from "../state/mail.svelte";
	import { errorText, t } from "../i18n/index.svelte";
	import { initials } from "../format";
	import { handleShortcut } from "../shortcuts";
	import { toast } from "@codeatlasdev/ui";
	import MailView from "./mail/MailView.svelte";
	import ContactsView from "./contacts/ContactsView.svelte";
	import SettingsView from "./settings/SettingsView.svelte";
	import ShortcutsDialog from "./ShortcutsDialog.svelte";

	let showShortcuts = $state(false);

	const items: { view: View; label: string; icon: typeof Mail }[] = $derived([
		{ view: "mail", label: t("nav.mail"), icon: Mail },
		{ view: "contacts", label: t("nav.contacts"), icon: UserRound },
		{ view: "settings", label: t("nav.settings"), icon: Settings },
	]);

	function go(view: View) {
		app.view = view;
		if (view !== "mail") compose.hide();
	}

	async function switchAccount(id: string) {
		if (id === app.account?.id) return;
		if (compose.active && !(await compose.closeAsking())) return;
		mail.stop();
		try {
			await app.openAccount(id);
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	function onKeydown(e: KeyboardEvent) {
		if (e.key === "?" && !isTyping(e)) {
			e.preventDefault();
			showShortcuts = true;
			return;
		}
		handleShortcut(e);
	}

	function isTyping(e: KeyboardEvent) {
		const el = e.target as HTMLElement | null;
		return !!el?.closest("input, textarea, select, [contenteditable=''], [contenteditable='true']");
	}

	$effect(() => {
		const show = () => (showShortcuts = true);
		window.addEventListener("lm:shortcuts", show);
		return () => window.removeEventListener("lm:shortcuts", show);
	});

	// Desktop: mailto: links from other apps open the editor.
	$effect(() => {
		if (!isDesktop) return;
		let unlisten: (() => void) | undefined;
		const open = (url: string) => /^mailto:/i.test(url) && void compose.mailto(url);
		void (async () => {
			const { invoke } = await import("@tauri-apps/api/core");
			const { listen } = await import("@tauri-apps/api/event");
			if (!startupHandled) {
				startupHandled = true;
				for (const url of await invoke<string[]>("startup_urls").catch(() => [] as string[])) open(url);
			}
			unlisten = await listen<string>("open-url", (e) => open(e.payload));
		})();
		return () => unlisten?.();
	});

	onDestroy(() => mail.stop());
</script>

<svelte:window onkeydown={onKeydown} />

<div class="lm-shell">
	<nav class="lm-rail" aria-label={t("nav.label")}>
		{#if !isDesktop}
			<img class="lm-rail__logo" src="./favicon.svg" alt={app.productName} width="28" height="28" />
		{/if}
		<Tooltip content={`${t("action.compose")} (C)`} side="right">
			<button type="button" class="lm-rail__compose" aria-label={t("action.compose")} onclick={() => (compose.active ? compose.show() : compose.open("new"))}>
				<SquarePen size={18} />
			</button>
		</Tooltip>
		{#each items as item (item.view)}
			<Tooltip content={item.label} side="right">
				<button
					type="button"
					class="lm-rail__item"
					class:lm-rail__item--active={app.view === item.view}
					aria-label={item.label}
					aria-current={app.view === item.view ? "page" : undefined}
					onclick={() => go(item.view)}
				>
					<item.icon size={19} />
					{#if item.view === "mail" && mail.inboxUnread > 0}
						<span class="lm-rail__badge">{mail.inboxUnread > 99 ? "99+" : mail.inboxUnread}</span>
					{/if}
				</button>
			</Tooltip>
		{/each}
		<div class="lm-spacer"></div>
		{#if updates.available}
			<Tooltip content={t("updates.available", { version: updates.available.version })} side="right">
				<button type="button" class="lm-rail__item lm-rail__update" aria-label={t("updates.available", { version: updates.available.version })} onclick={() => (updates.dialogOpen = true)}>
					<ArrowUpCircle size={19} />
				</button>
			</Tooltip>
		{/if}
		{#if !app.online}
			<Tooltip content={t("status.offline")} side="right">
				<span class="lm-rail__offline" role="status" aria-label={t("status.offline")}><WifiOff size={16} /></span>
			</Tooltip>
		{/if}
		<DropdownMenu side="right" align="end">
			{#snippet triggerChild(props)}
				<button {...props} type="button" class="lm-rail__account" aria-label={t("account.menu")}>
					<Avatar size="sm" fallback={initials({ email: app.account?.email ?? "", name: app.defaultIdentity?.name })} alt="" decorative />
				</button>
			{/snippet}
			<DropdownMenuLabel>{app.account?.email}</DropdownMenuLabel>
			{#if isDesktop}
				<DropdownMenuSeparator />
				{#each app.accounts as acc (acc.id)}
					<DropdownMenuItem onclick={() => switchAccount(acc.id)}>
						{#snippet icon()}{#if acc.id === app.account?.id}<Check size={14} />{:else}<span class="lm-icon-gap"></span>{/if}{/snippet}
						{acc.label || acc.email}
					</DropdownMenuItem>
				{/each}
				<DropdownMenuItem onclick={() => (app.editingAccount = null)}>
					{#snippet icon()}<Plus size={14} />{/snippet}
					{t("account.add")}
				</DropdownMenuItem>
			{/if}
			<DropdownMenuSeparator />
			<DropdownMenuItem onclick={() => (showShortcuts = true)}>{t("shortcuts.title")}</DropdownMenuItem>
			{#if !isDesktop}
				<DropdownMenuItem onclick={() => app.signOut()}>
					{#snippet icon()}<LogOut size={14} />{/snippet}
					{t("account.signOut")}
				</DropdownMenuItem>
			{/if}
		</DropdownMenu>
	</nav>

	<div class="lm-view">
		{#if app.view === "mail"}
			<MailView />
		{:else if app.view === "contacts"}
			<ContactsView />
		{:else}
			<SettingsView />
		{/if}
	</div>
</div>

<ShortcutsDialog bind:open={showShortcuts} />

<style>
	.lm-shell {
		flex: 1;
		min-height: 0;
		display: flex;
	}
	.lm-rail {
		width: var(--lm-rail-w);
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--ca-space-1-5);
		padding: var(--ca-space-3-5) 0;
		background: var(--ca-surface-sunken);
		border-right: 1px solid var(--ca-outline-subtle);
	}
	.lm-rail__logo {
		margin-bottom: var(--ca-space-2);
	}
	.lm-rail__compose,
	.lm-rail__item,
	.lm-rail__account {
		position: relative;
		width: var(--lm-rail-item);
		height: var(--lm-rail-item);
		display: grid;
		place-items: center;
		border: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		cursor: pointer;
	}
	.lm-rail__compose {
		background: var(--ca-brand-fill);
		color: var(--ca-brand-contrast);
		margin-bottom: var(--ca-space-2-5);
	}
	.lm-rail__compose:hover {
		background: var(--ca-brand-fill-hover);
	}
	.lm-rail__item:hover,
	.lm-rail__account:hover {
		background: var(--lm-row-hover);
		color: var(--ca-text-primary);
	}
	.lm-rail__item--active {
		color: var(--ca-text-primary);
		background: var(--ca-surface-base);
		box-shadow: inset var(--lm-accent) 0 0 var(--lm-signal);
	}
	.lm-rail__badge {
		position: absolute;
		top: 1px;
		left: calc(50% + var(--ca-space-0-5));
		min-width: var(--lm-icon);
		height: var(--lm-icon);
		padding: 0 var(--ca-space-1-5);
		box-sizing: border-box;
		background: var(--ca-brand-fill);
		color: var(--ca-brand-contrast);
		font-size: var(--ca-font-size-micro);
		font-weight: 750;
		line-height: var(--lm-icon);
		text-align: center;
		pointer-events: none;
	}
	.lm-rail__update {
		color: var(--ca-brand-text);
	}
	.lm-rail__offline {
		color: var(--lm-danger);
		padding: var(--ca-space-2);
	}
	.lm-view {
		flex: 1;
		min-width: 0;
		display: flex;
	}
	.lm-icon-gap {
		display: inline-block;
		width: var(--ca-space-4-5);
	}
	@media (max-width: 640px) {
		.lm-rail {
			--lm-rail-w: 46px;
		}
	}
</style>
