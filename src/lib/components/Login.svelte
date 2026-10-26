<script lang="ts">
	import { Alert, AuthCard, AuthLayout, Button, Stack, TextField } from "@codeatlasdev/ui";
	import { app } from "../state/app.svelte";
	import { errorText, t } from "../i18n/index.svelte";

	let email = $state("");
	let password = $state("");
	let busy = $state(false);
	let error = $state("");

	const domain = $derived(app.webConfig?.domain);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (busy) return;
		error = "";
		if (!email.trim() || !password) {
			error = t("login.missing");
			return;
		}
		busy = true;
		try {
			await app.signIn(email.trim(), password);
			password = "";
		} catch (err) {
			error = errorText(err);
		} finally {
			busy = false;
		}
	}
</script>

<AuthLayout logoName={app.productName} logoHref="./" panelTone="flat">
	{#snippet brand()}<img src="./favicon.svg" width="28" height="28" alt="" />{/snippet}
	{#snippet panel()}
		<div class="lm-login-panel">
			<p class="lm-login-panel__title">{app.productName}</p>
			<p>{t("login.tagline")}</p>
		</div>
	{/snippet}
	<AuthCard title={t("login.title")} subtitle={domain ? t("login.subtitleDomain", { domain }) : t("login.subtitle")}>
		<form onsubmit={submit} novalidate>
			<Stack gap={12}>
				<TextField
					label={t("login.email")}
					type={domain ? "text" : "email"}
					autocomplete="username"
					bind:value={email}
					placeholder={domain ? t("login.userPlaceholder", { domain }) : t("login.emailPlaceholder")}
					required
				/>
				<TextField label={t("login.password")} type="password" autocomplete="current-password" revealable bind:value={password} required />
				{#if error}
					<Alert severity="error">{error}</Alert>
				{/if}
				<Button variant="filled" type="submit" fullWidth loading={busy}>{t("login.submit")}</Button>
			</Stack>
		</form>
		{#snippet footer()}
			<span class="lm-hint">{t("login.privacy")}</span>
		{/snippet}
	</AuthCard>
</AuthLayout>

<style>
	.lm-login-panel {
		max-width: 22rem;
	}
	.lm-login-panel__title {
		font-size: var(--ca-font-size-h2);
		font-weight: 750;
		margin: 0 0 var(--ca-space-2-5);
	}
</style>
