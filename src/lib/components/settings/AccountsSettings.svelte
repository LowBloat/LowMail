<script lang="ts">
	import { Badge, Button, IconButton, toast } from "@codeatlasdev/ui";
	import { Pencil, Plus, Trash2 } from "@lucide/svelte";
	import { desktop, isMobile } from "../../api";
	import { app } from "../../state/app.svelte";
	import { mail } from "../../state/mail.svelte";
	import { confirm } from "../../state/dialogs.svelte";
	import { errorText, t } from "../../i18n/index.svelte";
	import type { AccountConfig } from "../../types";

	async function remove(acc: AccountConfig) {
		if (!(await confirm({ title: t("accounts.delete.title", { email: acc.email }), description: t("accounts.delete.body"), confirmLabel: t("action.delete"), destructive: true }))) return;
		try {
			await desktop.deleteAccount(acc.id);
			await app.reloadAccounts();
			if (app.account?.id === acc.id) {
				mail.stop();
				const next = app.accounts[0];
				if (next) await app.openAccount(next.id);
				else {
					app.signedIn = false;
					app.account = null;
					app.editingAccount = null;
				}
			}
		} catch (e) {
			toast.error(errorText(e));
		}
	}
</script>

<h2>{t("settings.accounts")}</h2>
<p class="lm-muted lm-intro">{t(isMobile ? "accounts.introApp" : "accounts.intro")}</p>
<ul class="lm-accounts">
	{#each app.accounts as acc (acc.id)}
		<li class="lm-account">
			<div class="lm-account__text">
				<span class="lm-account__name">{acc.label || acc.email}</span>
				<span class="lm-hint">{t("accounts.servers", { email: acc.email, imap: `${acc.imap.host}:${acc.imap.port}`, smtp: `${acc.smtp.host}:${acc.smtp.port}` })}</span>
			</div>
			{#if acc.id === app.account?.id}<Badge tone="brand" subtle>{t("accounts.current")}</Badge>{/if}
			<IconButton variant="text" size="sm" aria-label={t("action.edit")} onclick={() => (app.editingAccount = acc)}><Pencil size={14} /></IconButton>
			<IconButton variant="text" size="sm" aria-label={t("action.delete")} onclick={() => remove(acc)}><Trash2 size={14} /></IconButton>
		</li>
	{/each}
</ul>
<div>
	<Button variant="outlined" onclick={() => (app.editingAccount = null)}>
		{#snippet leading()}<Plus size={15} />{/snippet}
		{t("account.add")}
	</Button>
</div>

<style>
	.lm-intro {
		margin: -10px 0 0;
	}
	.lm-accounts {
		list-style: none;
		margin: 0;
		padding: 0;
		border: 1px solid var(--ca-outline-subtle);
	}
	.lm-account {
		display: flex;
		align-items: center;
		gap: var(--ca-space-3-5);
		padding: var(--ca-space-3-5) var(--ca-space-4);
		border-bottom: 1px solid var(--ca-outline-subtle);
	}
	.lm-account__text {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
	}
	.lm-account__name {
		font-weight: 600;
	}
</style>
