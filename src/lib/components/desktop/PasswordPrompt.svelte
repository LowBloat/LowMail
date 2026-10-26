<script lang="ts">
	import { Alert, AuthCard, Button, Stack, TextField } from "@codeatlasdev/ui";
	import { app } from "../../state/app.svelte";
	import { errorText, t } from "../../i18n/index.svelte";
	import type { AccountConfig } from "../../types";

	let { account }: { account: AccountConfig } = $props();

	let password = $state("");
	let busy = $state(false);
	let error = $state("");

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (!password) return;
		busy = true;
		error = "";
		try {
			await app.openAccount(account.id, password);
			password = "";
		} catch (err) {
			error = errorText(err);
		} finally {
			busy = false;
		}
	}
</script>

<div class="lm-password">
	<AuthCard title={t("account.passwordFor", { email: account.email })} subtitle={t("account.passwordWhy")}>
		<form onsubmit={submit}>
			<Stack gap={12}>
				<TextField label={t("login.password")} type="password" revealable bind:value={password} autocomplete="current-password" />
				{#if error}<Alert severity="error">{error}</Alert>{/if}
				<Button variant="filled" type="submit" fullWidth loading={busy}>{t("account.connect")}</Button>
				<Button variant="ghost" fullWidth onclick={() => ((app.passwordFor = null), (app.editingAccount = account))}>{t("account.edit")}</Button>
			</Stack>
		</form>
	</AuthCard>
</div>

<style>
	.lm-password {
		flex: 1;
		display: grid;
		place-items: center;
		padding: 24px;
	}
</style>
