<script lang="ts">
	import { Badge, Button, FormSection, Row, Stack, Switch, TextField, toast } from "@codeatlasdev/ui";
	import { Plus } from "@lucide/svelte";
	import { call } from "../../api";
	import { app } from "../../state/app.svelte";
	import { confirm } from "../../state/dialogs.svelte";
	import { errorText, t } from "../../i18n/index.svelte";
	import type { Identity } from "../../types";
	import Editor from "../compose/Editor.svelte";

	let editing = $state<Identity | null>(null);
	let version = $state(0);
	let saving = $state(false);

	function edit(i: Identity) {
		editing = { ...$state.snapshot(i) };
		version++;
	}

	function create() {
		editing = { id: "", name: app.defaultIdentity?.name ?? "", email: "", replyTo: null, bcc: null, signature: "", isDefault: false };
		version++;
	}

	async function save() {
		if (!editing) return;
		saving = true;
		try {
			const identity = { ...$state.snapshot(editing), replyTo: editing.replyTo?.trim() || null, bcc: editing.bcc?.trim() || null };
			await call("identities.save", { identity });
			await app.loadIdentities();
			editing = null;
			toast.success(t("identities.saved"));
		} catch (e) {
			toast.error(errorText(e));
		} finally {
			saving = false;
		}
	}

	async function remove(i: Identity) {
		if (!(await confirm({ title: t("identities.delete", { email: i.email }), confirmLabel: t("action.delete"), destructive: true }))) return;
		try {
			await call("identities.delete", { id: i.id });
			await app.loadIdentities();
			editing = null;
		} catch (e) {
			toast.error(errorText(e));
		}
	}
</script>

<h2>{t("settings.identities")}</h2>
<p class="lm-muted lm-intro">{t("identities.intro")}</p>

{#if editing}
	<FormSection title={editing.id ? t("identities.edit") : t("identities.new")} layout="stack">
		<Stack gap={12}>
			<TextField label={t("identities.name")} description={t("identities.nameHint")} bind:value={editing.name} />
			<TextField label={t("identities.email")} type="email" required bind:value={editing.email} />
			<TextField label={t("identities.replyTo")} type="email" value={editing.replyTo ?? ""} onchange={(v) => editing && (editing.replyTo = v)} />
			<TextField label={t("identities.bcc")} description={t("identities.bccHint")} type="email" value={editing.bcc ?? ""} onchange={(v) => editing && (editing.bcc = v)} />
			<div class="lm-sig">
				<span class="lm-sig__label">{t("identities.signature")}</span>
				<div class="lm-sig__editor">
					<Editor html={editing.signature} {version} minHeight={150} onchange={(h) => editing && (editing.signature = h.replace(/^(<p><br><\/p>|<br>)+$/, ""))} />
				</div>
			</div>
			<Switch label={t("identities.default")} bind:checked={editing.isDefault} />
			<Row gap={8} justify="space-between">
				<div>
					{#if editing.id && app.identities.length > 1}
						<Button variant="ghost" class="ca-btn-danger-ghost" onclick={() => editing && remove(editing)}>{t("action.delete")}</Button>
					{/if}
				</div>
				<Row gap={8}>
					<Button variant="ghost" onclick={() => (editing = null)}>{t("action.cancel")}</Button>
					<Button variant="filled" loading={saving} onclick={save}>{t("action.save")}</Button>
				</Row>
			</Row>
		</Stack>
	</FormSection>
{:else}
	<ul class="lm-id-list">
		{#each app.identities as i (i.id)}
			<li>
				<button type="button" class="lm-id" onclick={() => edit(i)}>
					<span class="lm-id__text">
						<span class="lm-id__name">{i.name || i.email}</span>
						<span class="lm-hint">{i.email}</span>
					</span>
					{#if i.isDefault}<Badge tone="brand" subtle>{t("identities.defaultBadge")}</Badge>{/if}
				</button>
			</li>
		{/each}
	</ul>
	<div>
		<Button variant="outlined" onclick={create}>
			{#snippet leading()}<Plus size={15} />{/snippet}
			{t("identities.add")}
		</Button>
	</div>
{/if}

<style>
	.lm-intro {
		margin: -10px 0 0;
	}
	.lm-id-list {
		list-style: none;
		margin: 0;
		padding: 0;
		border: 1px solid var(--ca-outline-subtle);
	}
	.lm-id {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 12px 14px;
		border: 0;
		border-bottom: 1px solid var(--ca-outline-subtle);
		background: transparent;
		color: var(--ca-text-primary);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.lm-id:hover {
		background: var(--lm-row-hover);
	}
	.lm-id__text {
		flex: 1;
		display: flex;
		flex-direction: column;
	}
	.lm-id__name {
		font-weight: 600;
	}
	.lm-sig {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.lm-sig__label {
		font-size: 13px;
		font-weight: 550;
	}
	.lm-sig__editor {
		display: flex;
		height: 230px;
	}
</style>
