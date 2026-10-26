<script lang="ts">
	import { onMount } from "svelte";
	import { Alert, Button, Checkbox, EmptyState, FormSection, IconButton, Row, SegmentedControl, Select, Spinner, Stack, Switch, TextField } from "@codeatlasdev/ui";
	import { ArrowDown, ArrowUp, Filter, Pencil, Plus, Trash2, X } from "@lucide/svelte";
	import { filters } from "../../state/filters.svelte";
	import { mail } from "../../state/mail.svelte";
	import { confirm } from "../../state/dialogs.svelte";
	import { t } from "../../i18n/index.svelte";
	import { folderPathLabel } from "../../format";
	import type { FilterAction, FilterCondition, FilterField, FilterOp, FilterRule } from "../../types";

	let editing = $state<FilterRule | null>(null);
	let editingIndex = $state(-1);

	onMount(() => void filters.load(true));

	const rules = $derived(filters.state?.filters.rules ?? []);

	const fieldOptions = $derived(
		(["from", "to", "cc", "recipient", "subject", "body", "header", "size"] as FilterField[]).map((value) => ({ value, label: t(`filters.field.${value}`) })),
	);
	const textOps = $derived((["contains", "notContains", "is", "isNot", "startsWith", "endsWith"] as FilterOp[]).map((value) => ({ value, label: t(`filters.op.${value}`) })));
	const sizeOps = $derived((["over", "under"] as FilterOp[]).map((value) => ({ value, label: t(`filters.op.${value}`) })));
	const actionOptions = $derived(
		["move", "copy", "markRead", "flag", "forward", "discard", "reject"].map((value) => ({ value, label: t(`filters.action.${value}`) })),
	);
	const folderOptions = $derived(mail.folders.filter((f) => f.selectable).map((f) => ({ value: f.path, label: folderPathLabel(f, mail.folders) })));

	/** A user folder, else Archive, else the inbox. */
	function defaultFolder() {
		return mail.folders.find((f) => f.selectable && !f.role)?.path ?? mail.folderByRole("archive")?.path ?? "INBOX";
	}

	function folderName(path: string) {
		const f = mail.folders.find((x) => x.path === path);
		return f ? folderPathLabel(f, mail.folders) : path;
	}

	function newRule(): FilterRule {
		return {
			id: crypto.randomUUID(),
			name: "",
			enabled: true,
			matchAll: true,
			conditions: [{ field: "from", header: "", op: "contains", value: "" }],
			actions: [{ type: "move", folder: defaultFolder() }],
			stop: true,
		};
	}

	function edit(i: number) {
		editingIndex = i;
		editing = i < 0 ? newRule() : structuredClone($state.snapshot(rules[i]));
	}

	function setField(c: FilterCondition, field: FilterField) {
		c.field = field;
		if (field === "size") c.op = "over";
		else if (c.op === "over" || c.op === "under") c.op = "contains";
	}

	function setAction(i: number, type: string) {
		if (!editing) return;
		const defaults: Record<string, FilterAction> = {
			move: { type: "move", folder: defaultFolder() },
			copy: { type: "copy", folder: defaultFolder() },
			markRead: { type: "markRead" },
			flag: { type: "flag" },
			forward: { type: "forward", to: "", keep: true },
			discard: { type: "discard" },
			reject: { type: "reject", message: "" },
		};
		editing.actions[i] = defaults[type];
	}

	function describe(rule: FilterRule) {
		const conds = rule.conditions.map((c) => `${t(`filters.field.${c.field}`)}${c.field === "header" ? ` ${c.header}` : ""} ${t(`filters.op.${c.op}`)} "${c.value}"`).join(rule.matchAll ? ` ${t("filters.and")} ` : ` ${t("filters.or")} `);
		const acts = rule.actions
			.map((a) => {
				if (a.type === "move" || a.type === "copy") return `${t(`filters.action.${a.type}`)} ${folderName(a.folder)}`;
				if (a.type === "forward") return `${t("filters.action.forward")} ${a.to}`;
				return t(`filters.action.${a.type}`);
			})
			.join(", ");
		return `${conds} → ${acts}`;
	}

	async function persist(next: FilterRule[]) {
		if (!filters.state) return false;
		return filters.save({ ...$state.snapshot(filters.state.filters), rules: next });
	}

	async function saveRule() {
		if (!editing) return;
		if (!editing.name.trim()) editing.name = t("filters.untitled");
		const next = [...$state.snapshot(rules)];
		if (editingIndex < 0) next.push($state.snapshot(editing));
		else next[editingIndex] = $state.snapshot(editing);
		if (await persist(next)) editing = null;
	}

	async function remove(i: number) {
		if (!(await confirm({ title: t("filters.delete", { name: rules[i].name }), confirmLabel: t("action.delete"), destructive: true }))) return;
		await persist(rules.filter((_, j) => j !== i));
	}

	async function toggle(i: number, on: boolean) {
		const next = $state.snapshot(rules);
		next[i].enabled = on;
		await persist(next);
	}

	async function moveRule(i: number, step: number) {
		const next = $state.snapshot(rules);
		const j = i + step;
		if (j < 0 || j >= next.length) return;
		[next[i], next[j]] = [next[j], next[i]];
		await persist(next);
	}
</script>

<h2>{t("settings.filters")}</h2>
<p class="lm-muted lm-intro">{t("filters.intro")}</p>

{#if filters.loading && !filters.state}
	<Spinner size={22} label={t("app.loading")} />
{:else if filters.error}
	<Alert severity="error" title={t("filters.unavailable")}>{filters.error}</Alert>
{:else if filters.state}
	{#if filters.state.foreignScript}
		<Alert severity="warning" title={t("filters.foreign.title", { name: filters.state.foreignScript })}>
			{t("filters.foreign.body")}
			{#if filters.state.foreignContent}
				<details><summary>{t("filters.foreign.show")}</summary><pre class="lm-script">{filters.state.foreignContent}</pre></details>
			{/if}
		</Alert>
	{/if}

	{#if editing}
		<FormSection title={editingIndex < 0 ? t("filters.new") : t("filters.edit")} layout="stack">
			<Stack gap={12}>
				<TextField label={t("filters.name")} bind:value={editing.name} placeholder={t("filters.namePlaceholder")} />
				<div class="lm-rule-block">
					<Row gap={10} align="center" wrap>
						<strong>{t("filters.when")}</strong>
						<SegmentedControl size="sm" value={editing.matchAll ? "all" : "any"} items={[{ value: "all", label: t("filters.matchAll") }, { value: "any", label: t("filters.matchAny") }]} onchange={(v) => editing && (editing.matchAll = v === "all")} />
					</Row>
					{#each editing.conditions as c, i (i)}
						<div class="lm-rule-row">
							<Select class="lm-rule-select" size="sm" value={c.field} options={fieldOptions} aria-label={t("filters.fieldLabel")} onchange={(v) => setField(c, v as FilterField)} />
							{#if c.field === "header"}
								<input class="lm-input" placeholder={t("filters.headerName")} aria-label={t("filters.headerName")} bind:value={c.header} />
							{/if}
							<Select class="lm-rule-select" size="sm" value={c.op} options={c.field === "size" ? sizeOps : textOps} aria-label={t("filters.opLabel")} onchange={(v) => (c.op = v as FilterOp)} />
							<input class="lm-input lm-rule-row__value" placeholder={c.field === "size" ? t("filters.sizeKb") : t("filters.value")} aria-label={t("filters.value")} bind:value={c.value} type={c.field === "size" ? "number" : "text"} />
							{#if editing.conditions.length > 1}
								<IconButton variant="text" size="sm" aria-label={t("action.remove")} onclick={() => editing?.conditions.splice(i, 1)}><X size={14} /></IconButton>
							{/if}
						</div>
					{/each}
					<button type="button" class="lm-add" onclick={() => editing?.conditions.push({ field: "subject", header: "", op: "contains", value: "" })}>+ {t("filters.addCondition")}</button>
				</div>
				<div class="lm-rule-block">
					<strong>{t("filters.then")}</strong>
					{#each editing.actions as a, i (i)}
						<div class="lm-rule-row">
							<Select class="lm-rule-select" size="sm" value={a.type} options={actionOptions} aria-label={t("filters.actionLabel")} onchange={(v) => setAction(i, v)} />
							{#if a.type === "move" || a.type === "copy"}
								<Select class="lm-rule-select lm-rule-select--wide" size="sm" value={a.folder} options={folderOptions} aria-label={t("filters.folder")} onchange={(v) => a.type === "move" || a.type === "copy" ? (a.folder = v) : null} />
							{:else if a.type === "forward"}
								<input class="lm-input lm-rule-row__value" type="email" placeholder={t("filters.forwardTo")} aria-label={t("filters.forwardTo")} bind:value={a.to} />
								<Checkbox label={t("filters.keepCopy")} bind:checked={a.keep} />
							{:else if a.type === "reject"}
								<input class="lm-input lm-rule-row__value" placeholder={t("filters.rejectMessage")} aria-label={t("filters.rejectMessage")} bind:value={a.message} />
							{/if}
							{#if editing.actions.length > 1}
								<IconButton variant="text" size="sm" aria-label={t("action.remove")} onclick={() => editing?.actions.splice(i, 1)}><X size={14} /></IconButton>
							{/if}
						</div>
					{/each}
					<button type="button" class="lm-add" onclick={() => editing?.actions.push({ type: "markRead" })}>+ {t("filters.addAction")}</button>
				</div>
				<Checkbox label={t("filters.stop")} bind:checked={editing.stop} />
				<Row gap={8} justify="flex-end">
					<Button variant="ghost" onclick={() => (editing = null)}>{t("action.cancel")}</Button>
					<Button variant="filled" loading={filters.saving} onclick={saveRule}>{t("action.save")}</Button>
				</Row>
			</Stack>
		</FormSection>
	{:else}
		{#if rules.length}
			<ul class="lm-rules">
				{#each rules as rule, i (rule.id)}
					<li class="lm-rule" class:lm-rule--off={!rule.enabled}>
						<Switch checked={rule.enabled} aria-label={t("filters.enabled")} onchange={(v) => toggle(i, v)} />
						<div class="lm-rule__text">
							<span class="lm-rule__name">{rule.name}</span>
							<span class="lm-hint lm-ellipsis">{describe(rule)}</span>
						</div>
						<IconButton variant="text" size="sm" aria-label={t("filters.up")} disabled={i === 0} onclick={() => moveRule(i, -1)}><ArrowUp size={14} /></IconButton>
						<IconButton variant="text" size="sm" aria-label={t("filters.down")} disabled={i === rules.length - 1} onclick={() => moveRule(i, 1)}><ArrowDown size={14} /></IconButton>
						<IconButton variant="text" size="sm" aria-label={t("action.edit")} onclick={() => edit(i)}><Pencil size={14} /></IconButton>
						<IconButton variant="text" size="sm" aria-label={t("action.delete")} onclick={() => remove(i)}><Trash2 size={14} /></IconButton>
					</li>
				{/each}
			</ul>
		{:else}
			<EmptyState icon={Filter} title={t("filters.empty")} description={t("filters.emptyHint")} />
		{/if}
		<div>
			<Button variant="outlined" onclick={() => edit(-1)}>
				{#snippet leading()}<Plus size={15} />{/snippet}
				{t("filters.new")}
			</Button>
		</div>
	{/if}
{/if}

<style>
	.lm-intro {
		margin: -10px 0 0;
	}
	.lm-script {
		max-height: 260px;
		overflow: auto;
		font-size: 12px;
		white-space: pre-wrap;
	}
	.lm-rule-block {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 12px;
		border: 1px solid var(--ca-outline-subtle);
		background: var(--ca-surface-sunken);
	}
	.lm-rule-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
	}
	.lm-rule-row :global(.lm-rule-select) {
		flex: 0 0 180px;
		width: 180px;
	}
	.lm-rule-row :global(.lm-rule-select--wide) {
		flex-basis: 240px;
		width: 240px;
	}
	.lm-rule-row__value {
		flex: 1;
		min-width: 180px;
	}
	.lm-add {
		align-self: flex-start;
		border: 0;
		padding: 0;
		background: transparent;
		color: var(--ca-brand-text);
		font: inherit;
		font-size: 13px;
		cursor: pointer;
	}
	.lm-rules {
		list-style: none;
		margin: 0;
		padding: 0;
		border: 1px solid var(--ca-outline-subtle);
	}
	.lm-rule {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 10px 12px;
		border-bottom: 1px solid var(--ca-outline-subtle);
	}
	.lm-rule--off .lm-rule__text {
		opacity: 0.6;
	}
	.lm-rule__text {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
	}
	.lm-rule__name {
		font-weight: 600;
	}
</style>
