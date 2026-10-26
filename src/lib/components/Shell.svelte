<script lang="ts">
	import { onDestroy } from "svelte";
	import { Avatar, DropdownMenu, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, Tooltip } from "@codeatlasdev/ui";
	import { Check, LogOut, Mail, Plus, Settings, SquarePen, UserRound, WifiOff } from "@lucide/svelte";
	import { isDesktop } from "../api";
	import { app, type View } from "../state/app.svelte";
	import { compose } from "../state/compose.svelte";
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
		gap: 4px;
		padding: 10px 0;
		background: var(--ca-surface-sunken);
		border-right: 1px solid var(--ca-outline-subtle);
	}
	.lm-rail__logo {
		margin-bottom: 6px;
	}
	.lm-rail__compose,
	.lm-rail__item,
	.lm-rail__account {
		position: relative;
		width: 38px;
		height: 38px;
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
		margin-bottom: 8px;
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
		box-shadow: inset 3px 0 0 var(--lm-signal);
	}
	.lm-rail__badge {
		position: absolute;
		top: 1px;
		left: 21px;
		min-width: 16px;
		height: 16px;
		padding: 0 4px;
		box-sizing: border-box;
		background: var(--ca-brand-fill);
		color: var(--ca-brand-contrast);
		font-size: 10px;
		font-weight: 750;
		line-height: 16px;
		text-align: center;
		pointer-events: none;
	}
	.lm-rail__offline {
		color: var(--lm-danger);
		padding: 6px;
	}
	.lm-view {
		flex: 1;
		min-width: 0;
		display: flex;
	}
	.lm-icon-gap {
		display: inline-block;
		width: 14px;
	}
	@media (max-width: 640px) {
		.lm-rail {
			--lm-rail-w: 46px;
		}
	}
</style>
