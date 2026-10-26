<script lang="ts">
	/**
	 * Adds or edits a desktop account. A new account needs only the address
	 * and password: the servers are guessed and checked, and can be adjusted
	 * under "Server settings".
	 */
	import { Alert, Button, Checkbox, Dialog, FormSection, NumberField, Row, Select, Stack, Switch, TextField, toast } from "@codeatlasdev/ui";
	import { desktop } from "../../api";
	import { app } from "../../state/app.svelte";
	import { errorDetail, errorText, t } from "../../i18n/index.svelte";
	import { isEmail } from "../../format";
	import type { AccountConfig, Endpoint, Security } from "../../types";

	let { account, first = false, dialog = false }: { account: AccountConfig | null; first?: boolean; dialog?: boolean } = $props();

	function blank(): AccountConfig {
		const ep = (port: number, security: Security): Endpoint => ({ host: "", port, security, acceptInvalidCerts: false });
		return { id: "", label: "", name: "", email: "", username: "", imap: ep(993, "tls"), smtp: ep(587, "startTls"), sieve: ep(4190, "startTls"), smtpAuth: true };
	}

	// The form edits a copy taken when the editor opens.
	const initial = (() => account)();
	let form = $state<AccountConfig>(initial ? structuredClone($state.snapshot(initial)) : blank());
	let password = $state("");
	let advanced = $state(!!initial);
	let sieveOn = $state(initial ? initial.sieve !== null : true);
	let busy = $state(false);
	let error = $state("");
	let detail = $state("");
	/** The address the servers were last discovered for. */
	let discoveredFor = $state("");
	const editing = !!initial;

	const securityOptions = $derived([
		{ value: "tls", label: t("account.security.tls") },
		{ value: "startTls", label: t("account.security.starttls") },
		{ value: "none", label: t("account.security.none") },
	]);

	function defaultPort(kind: "imap" | "smtp" | "sieve", security: Security) {
		if (kind === "imap") return security === "tls" ? 993 : 143;
		if (kind === "smtp") return security === "tls" ? 465 : security === "startTls" ? 587 : 25;
		return 4190;
	}

	function setSecurity(kind: "imap" | "smtp" | "sieve", value: string) {
		const ep = kind === "sieve" ? form.sieve : form[kind];
		if (!ep) return;
		const before = defaultPort(kind, ep.security);
		ep.security = value as Security;
		if (ep.port === before) ep.port = defaultPort(kind, ep.security);
	}

	async function discover() {
		const res = await desktop.discover(form.email.trim());
		form.imap = res.imap;
		form.smtp = res.smtp;
		form.sieve = res.sieve ?? form.sieve;
		sieveOn = res.sieve !== null;
		discoveredFor = form.email.trim();
	}

	function close() {
		app.editingAccount = undefined;
	}

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		error = "";
		detail = "";
		const email = form.email.trim();
		if (!isEmail(email)) {
			error = t("account.invalidEmail");
			return;
		}
		if (!editing && !password) {
			error = t("account.passwordRequired");
			return;
		}
		busy = true;
		try {
			form.email = email;
			if (!form.username.trim()) form.username = email;
			if (!form.imap.host || (!advanced && discoveredFor !== email)) await discover();
			const toSave: AccountConfig = { ...$state.snapshot(form), sieve: sieveOn ? $state.snapshot(form.sieve) : null };
			if (toSave.sieve && !toSave.sieve.host) toSave.sieve.host = toSave.imap.host;
			await desktop.testAccount(toSave, password || null);
			const saved = await desktop.saveAccount(toSave, password || null);
			await app.reloadAccounts();
			toast.success(editing ? t("account.saved") : t("account.added", { email: saved.email }));
			close();
			if (!editing || app.account?.id === saved.id) await app.openAccount(saved.id);
		} catch (err) {
			error = errorText(err);
			detail = errorDetail(err) ?? "";
			advanced = true;
		} finally {
			busy = false;
		}
	}
</script>

{#snippet fields()}
	<form id="lm-account-form" onsubmit={submit}>
		<Stack gap={12}>
			<TextField label={t("account.name")} description={t("account.nameHint")} bind:value={form.name} autocomplete="name" />
			<TextField label={t("account.email")} type="email" bind:value={form.email} autocomplete="email" required />
			<TextField
				label={t("account.password")}
				type="password"
				revealable
				bind:value={password}
				autocomplete="current-password"
				description={editing ? t("account.passwordKeep") : t("account.passwordStored")}
			/>
			<Checkbox label={t("account.serverSettings")} bind:checked={advanced} />
			{#if advanced}
				<FormSection title={t("account.incoming")} layout="stack">
					<Row gap={10} wrap>
						<div class="lm-grow"><TextField label={t("account.host")} bind:value={form.imap.host} placeholder="imap.example.com" /></div>
						<div class="lm-port"><NumberField label={t("account.port")} min={1} max={65535} bind:value={form.imap.port} /></div>
						<Select label={t("account.security")} value={form.imap.security} options={securityOptions} onchange={(v) => setSecurity("imap", v)} />
					</Row>
					<TextField label={t("account.username")} description={t("account.usernameHint")} bind:value={form.username} />
				</FormSection>
				<FormSection title={t("account.outgoing")} layout="stack">
					<Row gap={10} wrap>
						<div class="lm-grow"><TextField label={t("account.host")} bind:value={form.smtp.host} placeholder="smtp.example.com" /></div>
						<div class="lm-port"><NumberField label={t("account.port")} min={1} max={65535} bind:value={form.smtp.port} /></div>
						<Select label={t("account.security")} value={form.smtp.security} options={securityOptions} onchange={(v) => setSecurity("smtp", v)} />
					</Row>
					<Switch label={t("account.smtpAuth")} bind:checked={form.smtpAuth} />
				</FormSection>
				<FormSection title={t("account.sieve")} description={t("account.sieveHint")} layout="stack">
					<Switch label={t("account.sieveOn")} bind:checked={sieveOn} />
					{#if sieveOn && form.sieve}
						<Row gap={10} wrap>
							<div class="lm-grow"><TextField label={t("account.host")} bind:value={form.sieve.host} placeholder={form.imap.host} /></div>
							<div class="lm-port"><NumberField label={t("account.port")} min={1} max={65535} bind:value={form.sieve.port} /></div>
							<Select label={t("account.security")} value={form.sieve.security} options={securityOptions} onchange={(v) => setSecurity("sieve", v)} />
						</Row>
					{/if}
				</FormSection>
				<Checkbox
					label={t("account.invalidCerts")}
					checked={form.imap.acceptInvalidCerts}
					onchange={(c) => {
						form.imap.acceptInvalidCerts = c;
						form.smtp.acceptInvalidCerts = c;
						if (form.sieve) form.sieve.acceptInvalidCerts = c;
					}}
				/>
				<TextField label={t("account.label")} description={t("account.labelHint")} bind:value={form.label} />
			{/if}
			{#if error}
				<Alert severity="error" title={error}>{detail}</Alert>
			{/if}
		</Stack>
	</form>
{/snippet}

{#if dialog}
	<Dialog open title={editing ? t("account.edit") : t("account.add")} size="lg" onOpenChange={(o) => !o && close()}>
		{@render fields()}
		{#snippet actions()}
			<Row gap={8} justify="flex-end">
				<Button variant="ghost" onclick={close}>{t("action.cancel")}</Button>
				<Button variant="filled" loading={busy} onclick={() => (document.getElementById("lm-account-form") as HTMLFormElement | null)?.requestSubmit()}>{editing ? t("action.save") : t("account.connect")}</Button>
			</Row>
		{/snippet}
	</Dialog>
{:else}
	<div class="lm-welcome">
		<div class="lm-welcome__card">
			<img src="./favicon.svg" width="44" height="44" alt="" />
			<h1>{first ? t("account.welcome") : t("account.add")}</h1>
			<p class="lm-muted">{t("account.welcomeHint")}</p>
			{@render fields()}
			<Row gap={8} justify="flex-end">
				{#if !first}<Button variant="ghost" onclick={close}>{t("action.cancel")}</Button>{/if}
				<Button variant="filled" loading={busy} onclick={() => (document.getElementById("lm-account-form") as HTMLFormElement | null)?.requestSubmit()}>{t("account.connect")}</Button>
			</Row>
		</div>
	</div>
{/if}

<style>
	.lm-welcome {
		flex: 1;
		overflow: auto;
		display: flex;
		justify-content: center;
		padding: 40px 16px;
	}
	.lm-welcome__card {
		width: 100%;
		max-width: 560px;
		display: flex;
		flex-direction: column;
		gap: 14px;
	}
	.lm-welcome__card h1 {
		margin: 6px 0 0;
		font-size: 24px;
	}
	.lm-welcome__card p {
		margin: 0 0 6px;
	}
	.lm-grow {
		flex: 1;
		min-width: 200px;
	}
	.lm-port {
		width: 110px;
	}
</style>
