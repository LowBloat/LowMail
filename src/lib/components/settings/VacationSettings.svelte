<script lang="ts">
	import { onMount } from "svelte";
	import { Alert, Button, FormSection, NumberField, Row, Spinner, Stack, Switch, TextArea, TextField } from "@codeatlasdev/ui";
	import { app } from "../../state/app.svelte";
	import { filters } from "../../state/filters.svelte";
	import { t } from "../../i18n/index.svelte";
	import type { Vacation } from "../../types";

	let form = $state<Vacation | null>(null);
	let addresses = $state("");

	onMount(async () => {
		await filters.load(true);
		const v = filters.state?.filters.vacation;
		form = v
			? { ...$state.snapshot(v), subject: v.subject || t("vacation.defaultSubject"), body: v.body || t("vacation.defaultBody") }
			: null;
		addresses = form?.addresses.join(", ") ?? "";
	});

	async function save() {
		if (!form || !filters.state) return;
		const vacation: Vacation = {
			...$state.snapshot(form),
			days: Math.max(1, Math.round(form.days || 1)),
			start: form.start || null,
			end: form.end || null,
			addresses: addresses.split(/[,;\s]+/).map((a) => a.trim()).filter(Boolean),
		};
		await filters.save({ ...$state.snapshot(filters.state.filters), vacation });
	}
</script>

<h2>{t("settings.vacation")}</h2>
<p class="lm-muted lm-intro">{t("vacation.intro")}</p>

{#if filters.loading && !form}
	<Spinner size={22} label={t("app.loading")} />
{:else if filters.error}
	<Alert severity="error" title={t("filters.unavailable")}>{filters.error}</Alert>
{:else if form}
	{#if form.enabled}
		<Alert severity="info">{t("vacation.active")}</Alert>
	{/if}
	<FormSection layout="stack">
		<Stack gap={12}>
			<Switch label={t("vacation.enabled")} bind:checked={form.enabled} />
			<Row gap={12} wrap>
				<TextField label={t("vacation.start")} type="date" value={form.start ?? ""} onchange={(v) => form && (form.start = v || null)} />
				<TextField label={t("vacation.end")} type="date" value={form.end ?? ""} onchange={(v) => form && (form.end = v || null)} />
			</Row>
			<TextField label={t("vacation.subject")} bind:value={form.subject} />
			<TextArea label={t("vacation.body")} bind:value={form.body} rows={7} />
			<NumberField label={t("vacation.days")} description={t("vacation.daysHint")} min={1} max={30} bind:value={form.days} />
			<TextField label={t("vacation.addresses")} description={t("vacation.addressesHint", { email: app.account?.email ?? "" })} bind:value={addresses} />
			<Row justify="flex-end">
				<Button variant="filled" loading={filters.saving} onclick={save}>{t("action.save")}</Button>
			</Row>
		</Stack>
	</FormSection>
{/if}

<style>
	.lm-intro {
		margin: -10px 0 0;
	}
</style>
