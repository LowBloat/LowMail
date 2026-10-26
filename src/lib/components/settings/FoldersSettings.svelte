<script lang="ts">
	import { Button, Switch, toast } from "@codeatlasdev/ui";
	import { Plus } from "@lucide/svelte";
	import { call } from "../../api";
	import { mail } from "../../state/mail.svelte";
	import { confirm, prompt } from "../../state/dialogs.svelte";
	import { errorText, t } from "../../i18n/index.svelte";
	import { folderLabel } from "../../format";
	import type { Folder } from "../../types";

	async function run(work: () => Promise<unknown>) {
		try {
			await work();
			await mail.loadFolders();
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	async function create(parent: Folder | null) {
		const name = await prompt({ title: parent ? t("folders.newSub", { name: folderLabel(parent) }) : t("folders.new"), label: t("folders.name"), confirmLabel: t("action.create") });
		if (name) await run(() => call("folders.create", { parent: parent?.path ?? null, name }));
	}

	async function rename(f: Folder) {
		const name = await prompt({ title: t("folders.rename"), label: t("folders.name"), value: f.name, confirmLabel: t("action.rename") });
		if (name && name !== f.name) await run(() => call("folders.rename", { path: f.path, name }));
	}

	async function move(f: Folder, parent: string) {
		await run(() => call("folders.rename", { path: f.path, name: f.name, parent: parent || null }));
	}

	async function remove(f: Folder) {
		if (await confirm({ title: t("folders.delete.title", { name: folderLabel(f) }), description: t("folders.delete.body"), confirmLabel: t("action.delete"), destructive: true })) {
			await run(() => call("folders.delete", { path: f.path }));
		}
	}

	async function subscribe(f: Folder, on: boolean) {
		await run(() => call("folders.subscribe", { path: f.path, subscribed: on }));
	}

	function parents(f: Folder) {
		const delim = f.delimiter ?? "/";
		return mail.folders.filter((p) => p.path !== f.path && !p.path.startsWith(f.path + delim) && p.canHaveChildren && p.role !== "inbox");
	}
</script>

<h2>{t("settings.folders")}</h2>
<p class="lm-muted lm-intro">{t("folders.intro")}</p>
<div>
	<Button variant="outlined" onclick={() => create(null)}>
		{#snippet leading()}<Plus size={15} />{/snippet}
		{t("folders.new")}
	</Button>
</div>
<table class="lm-ftable">
	<thead>
		<tr>
			<th>{t("folders.name")}</th>
			<th class="lm-ftable__num">{t("folders.messages")}</th>
			<th>{t("folders.subscribed")}</th>
			<th><span class="ca-sr-only">{t("folders.actions")}</span></th>
		</tr>
	</thead>
	<tbody>
		{#each mail.folders as f (f.path)}
			<tr>
				<td style:padding-left="{10 + f.depth * 16}px">
					<span class:lm-strong={!!f.role}>{folderLabel(f)}</span>
				</td>
				<td class="lm-ftable__num">{f.total}</td>
				<td>
					{#if f.role !== "inbox"}
						<Switch checked={f.subscribed} aria-label={t("folders.subscribed")} onchange={(v) => subscribe(f, v)} />
					{/if}
				</td>
				<td class="lm-ftable__actions">
					{#if f.canHaveChildren}<button type="button" class="lm-link" onclick={() => create(f)}>{t("folders.newSubShort")}</button>{/if}
					{#if !f.role}
						<button type="button" class="lm-link" onclick={() => rename(f)}>{t("action.rename")}</button>
						<select class="lm-input lm-ftable__move" aria-label={t("folders.moveInto")} value="" onchange={(e) => move(f, (e.currentTarget as HTMLSelectElement).value)}>
							<option value="" disabled>{t("folders.moveInto")}</option>
							{#if f.parent}<option value="">{t("folders.topLevel")}</option>{/if}
							{#each parents(f) as p (p.path)}
								{#if p.path !== f.parent}<option value={p.path}>{p.displayPath}</option>{/if}
							{/each}
						</select>
						<button type="button" class="lm-link lm-link--danger" onclick={() => remove(f)}>{t("action.delete")}</button>
					{/if}
				</td>
			</tr>
		{/each}
	</tbody>
</table>

<style>
	.lm-intro {
		margin: -10px 0 0;
	}
	.lm-ftable {
		width: 100%;
		border-collapse: collapse;
		font-size: 13px;
	}
	.lm-ftable th {
		text-align: left;
		font-weight: 600;
		color: var(--ca-text-secondary);
		padding: 6px 10px;
		border-bottom: 1px solid var(--ca-outline-default);
	}
	.lm-ftable td {
		padding: 6px 10px;
		border-bottom: 1px solid var(--ca-outline-subtle);
	}
	.lm-ftable__num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	.lm-ftable__actions {
		display: flex;
		align-items: center;
		gap: 12px;
		flex-wrap: wrap;
	}
	.lm-ftable__move {
		height: 26px;
		max-width: 150px;
	}
	.lm-strong {
		font-weight: 600;
	}
	.lm-link {
		border: 0;
		padding: 0;
		background: transparent;
		color: var(--ca-brand-text);
		font: inherit;
		cursor: pointer;
	}
	.lm-link--danger {
		color: var(--lm-danger);
	}
</style>
