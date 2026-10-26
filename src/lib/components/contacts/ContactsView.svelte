<script lang="ts">
	import { onMount } from "svelte";
	import { Avatar, Button, EmptyState, IconButton, Stack, TextArea, TextField, Tooltip, toast } from "@codeatlasdev/ui";
	import { ArrowLeft, Download, Mail, Plus, Trash2, Upload, UserRound, Users, X } from "@lucide/svelte";
	import { call, saveText } from "../../api";
	import { compose } from "../../state/compose.svelte";
	import { confirm } from "../../state/dialogs.svelte";
	import { errorText, t } from "../../i18n/index.svelte";
	import { initials } from "../../format";
	import type { Contact } from "../../types";

	let contacts = $state<Contact[]>([]);
	let groups = $state<string[]>([]);
	let group = $state<string | "*all" | "*collected">("*all");
	let query = $state("");
	let selectedId = $state<string | null>(null);
	let editing = $state<Contact | null>(null);
	let saving = $state(false);
	let loading = $state(true);
	let importInput: HTMLInputElement | undefined = $state();
	let width = $state(1200);
	/** On phones the open contact replaces the list, as in the mail view. */
	const phone = $derived(width < 680);

	function blank(): Contact {
		return { id: "", name: "", emails: [""], phones: [], organization: "", jobTitle: "", notes: "", birthday: null, groups: group.startsWith("*") ? [] : [group], collected: false, updatedAt: 0 };
	}

	async function load() {
		loading = true;
		try {
			contacts = await call<Contact[]>("contacts.list", { query: "", includeCollected: true });
			groups = await call<string[]>("contacts.groups");
		} catch (e) {
			toast.error(errorText(e));
		} finally {
			loading = false;
		}
	}

	onMount(load);

	const shown = $derived.by(() => {
		const q = query.trim().toLowerCase();
		return contacts.filter((c) => {
			if (group === "*collected" ? !c.collected : group === "*all" ? c.collected : !c.groups.includes(group)) return false;
			if (!q) return true;
			return c.name.toLowerCase().includes(q) || c.emails.some((e) => e.toLowerCase().includes(q)) || c.organization.toLowerCase().includes(q) || c.phones.some((p) => p.includes(q));
		});
	});

	const collectedCount = $derived(contacts.filter((c) => c.collected).length);

	function select(c: Contact) {
		selectedId = c.id;
		editing = { ...$state.snapshot(c), emails: c.emails.length ? [...c.emails] : [""], phones: [...c.phones], groups: [...c.groups] };
	}

	function create() {
		selectedId = null;
		editing = blank();
	}

	async function save() {
		if (!editing) return;
		saving = true;
		try {
			const contact = { ...$state.snapshot(editing), collected: false, emails: editing.emails.filter((e) => e.trim()), phones: editing.phones.filter((p) => p.trim()) };
			const saved = await call<Contact>("contacts.save", { contact });
			await load();
			selectedId = saved.id;
			const fresh = contacts.find((c) => c.id === saved.id);
			if (fresh) select(fresh);
			toast.success(t("contacts.saved"));
		} catch (e) {
			toast.error(errorText(e));
		} finally {
			saving = false;
		}
	}

	async function remove(c: Contact) {
		if (!(await confirm({ title: t("contacts.delete.title", { name: c.name || c.emails[0] }), confirmLabel: t("action.delete"), destructive: true }))) return;
		try {
			await call("contacts.delete", { ids: [c.id] });
			editing = null;
			selectedId = null;
			await load();
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	async function importFile(file: File) {
		try {
			const text = await file.text();
			const res = await call<{ added: number; skipped: number }>("contacts.import", { text, group: group.startsWith("*") ? null : group });
			toast.success(t("contacts.imported", { added: res.added, skipped: res.skipped }));
			await load();
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	async function exportAll() {
		try {
			const res = await call<{ vcard: string }>("contacts.export", {});
			await saveText("contatos.vcf", res.vcard, "text/vcard");
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	function write(c: Contact) {
		if (!c.emails[0]) return;
		void compose.open("new", null, { to: [{ name: c.name || null, email: c.emails[0] }] });
	}

	let groupsText = $derived(editing?.groups.join(", ") ?? "");
</script>

<div class="lm-contacts" bind:clientWidth={width}>
	<aside class="lm-contacts__groups">
		<div class="lm-bar"><span class="lm-bar__title">{t("contacts.title")}</span></div>
		<nav class="lm-contacts__nav">
			<button type="button" class:active={group === "*all"} onclick={() => (group = "*all")}><Users size={15} /> {t("contacts.all")}</button>
			{#each groups as g (g)}
				<button type="button" class:active={group === g} onclick={() => (group = g)}><span class="lm-contacts__dot"></span> {g}</button>
			{/each}
			<button type="button" class:active={group === "*collected"} onclick={() => (group = "*collected")}><UserRound size={15} /> {t("contacts.collected")} <span class="lm-hint">{collectedCount}</span></button>
		</nav>
	</aside>

	<section class="lm-pane lm-contacts__list" class:lm-contacts__list--full={phone} hidden={phone && !!editing}>
		<div class="lm-bar">
			<input class="lm-input lm-contacts__search" type="search" placeholder={t("contacts.search")} aria-label={t("contacts.search")} bind:value={query} />
			<Tooltip content={t("contacts.new")}><IconButton variant="filled" size="sm" aria-label={t("contacts.new")} onclick={create}><Plus size={16} /></IconButton></Tooltip>
			<Tooltip content={t("contacts.import")}><IconButton variant="text" size="sm" aria-label={t("contacts.import")} onclick={() => importInput?.click()}><Upload size={16} /></IconButton></Tooltip>
			<Tooltip content={t("contacts.export")}><IconButton variant="text" size="sm" aria-label={t("contacts.export")} onclick={exportAll}><Download size={16} /></IconButton></Tooltip>
		</div>
		<ul class="lm-scroll lm-contacts__items">
			{#each shown as c (c.id)}
				<li>
					<button type="button" class="lm-contact" class:lm-contact--active={selectedId === c.id} onclick={() => select(c)}>
						<Avatar size="sm" fallback={initials({ name: c.name, email: c.emails[0] ?? "" })} alt="" decorative />
						<span class="lm-contact__text">
							<span class="lm-ellipsis lm-contact__name">{c.name || c.emails[0]}</span>
							{#if c.name && c.emails[0]}<span class="lm-ellipsis lm-hint">{c.emails[0]}</span>{/if}
						</span>
					</button>
				</li>
			{:else}
				{#if !loading}
					<li><EmptyState icon={Users} title={query ? t("contacts.noResults") : t("contacts.empty")} description={query ? "" : t("contacts.emptyHint")} /></li>
				{/if}
			{/each}
		</ul>
	</section>

	<section class="lm-pane lm-contacts__detail" hidden={phone && !editing}>
		{#if editing}
			<div class="lm-bar">
				{#if phone}
					<IconButton variant="text" size="sm" aria-label={t("action.back")} onclick={() => ((editing = null), (selectedId = null))}><ArrowLeft size={17} /></IconButton>
				{/if}
				<span class="lm-bar__title">{editing.id ? editing.name || editing.emails[0] : t("contacts.new")}</span>
				<span class="lm-spacer"></span>
				{#if editing.id && editing.emails[0]}
					<Button variant="ghost" size="sm" onclick={() => editing && write(editing)}>{#snippet leading()}<Mail size={15} />{/snippet}{t("contacts.write")}</Button>
				{/if}
				{#if editing.id}
					<Tooltip content={t("action.delete")}><IconButton variant="text" size="sm" aria-label={t("action.delete")} onclick={() => editing && remove(editing)}><Trash2 size={16} /></IconButton></Tooltip>
				{/if}
			</div>
			<form class="lm-scroll lm-contacts__form" onsubmit={(e) => (e.preventDefault(), save())}>
				<Stack gap={12}>
					<TextField label={t("contacts.name")} bind:value={editing.name} />
					{#each editing.emails as _, i (i)}
						<div class="lm-contacts__multi">
							<TextField label={t("contacts.email")} hideLabel={i > 0} type="email" bind:value={editing.emails[i]} />
							{#if editing.emails.length > 1}
								<IconButton variant="text" size="sm" aria-label={t("action.remove")} onclick={() => editing && editing.emails.splice(i, 1)}><X size={14} /></IconButton>
							{/if}
						</div>
					{/each}
					<button type="button" class="lm-link-btn" onclick={() => editing?.emails.push("")}>+ {t("contacts.addEmail")}</button>
					{#each editing.phones as _, i (i)}
						<div class="lm-contacts__multi">
							<TextField label={t("contacts.phone")} hideLabel={i > 0} type="tel" bind:value={editing.phones[i]} />
							<IconButton variant="text" size="sm" aria-label={t("action.remove")} onclick={() => editing && editing.phones.splice(i, 1)}><X size={14} /></IconButton>
						</div>
					{/each}
					<button type="button" class="lm-link-btn" onclick={() => editing?.phones.push("")}>+ {t("contacts.addPhone")}</button>
					<TextField label={t("contacts.organization")} bind:value={editing.organization} />
					<TextField label={t("contacts.jobTitle")} bind:value={editing.jobTitle} />
					<TextField label={t("contacts.birthday")} type="date" value={editing.birthday ?? ""} onchange={(v) => editing && (editing.birthday = v || null)} />
					<TextField
						label={t("contacts.groups")}
						description={t("contacts.groupsHint")}
						value={groupsText}
						onchange={(v) => editing && (editing.groups = v.split(",").map((g) => g.trim()).filter(Boolean))}
					/>
					<TextArea label={t("contacts.notes")} bind:value={editing.notes} rows={4} />
					<div class="lm-contacts__actions">
						<Button variant="ghost" onclick={() => ((editing = null), (selectedId = null))}>{t("action.cancel")}</Button>
						<Button variant="filled" type="submit" loading={saving}>{t("action.save")}</Button>
					</div>
				</Stack>
			</form>
		{:else}
			<div class="lm-contacts__empty">
				<EmptyState icon={UserRound} title={t("contacts.pick")} description={t("contacts.pickHint")}>
					{#snippet actions()}<Button variant="outlined" size="sm" onclick={create}>{t("contacts.new")}</Button>{/snippet}
				</EmptyState>
			</div>
		{/if}
	</section>
</div>

<input
	bind:this={importInput}
	type="file"
	accept=".vcf,.vcard,.csv,text/vcard,text/csv"
	hidden
	onchange={(e) => {
		const input = e.currentTarget as HTMLInputElement;
		const f = input.files?.[0];
		if (f) void importFile(f);
		input.value = "";
	}}
/>

<style>
	.lm-contacts {
		flex: 1;
		min-width: 0;
		display: flex;
	}
	.lm-contacts__groups {
		width: 220px;
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		background: var(--ca-surface-sunken);
		border-right: 1px solid var(--ca-outline-subtle);
	}
	.lm-contacts__nav {
		display: flex;
		flex-direction: column;
		padding: 6px 0;
		overflow: auto;
	}
	.lm-contacts__nav button {
		display: flex;
		align-items: center;
		gap: 8px;
		height: 32px;
		padding: 0 12px;
		border: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.lm-contacts__nav button:hover {
		background: var(--lm-row-hover);
		color: var(--ca-text-primary);
	}
	.lm-contacts__nav button.active {
		background: var(--ca-surface-base);
		color: var(--ca-text-primary);
		font-weight: 600;
		box-shadow: inset 3px 0 0 var(--lm-signal);
	}
	.lm-contacts__dot {
		width: 8px;
		height: 8px;
		margin: 0 3px;
		background: var(--ca-outline-strong);
	}
	.lm-contacts__list {
		width: 340px;
		flex-shrink: 0;
		border-right: 1px solid var(--ca-outline-subtle);
	}
	.lm-contacts__search {
		flex: 1;
	}
	.lm-contacts__items {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.lm-contact {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 8px 12px;
		border: 0;
		border-bottom: 1px solid var(--ca-outline-subtle);
		background: transparent;
		color: var(--ca-text-primary);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.lm-contact:hover {
		background: var(--lm-row-hover);
	}
	.lm-contact--active {
		background: var(--lm-row-open);
	}
	.lm-contact__text {
		min-width: 0;
		display: flex;
		flex-direction: column;
	}
	.lm-contact__name {
		font-weight: 600;
	}
	.lm-contacts__detail {
		flex: 1;
	}
	.lm-contacts__form {
		padding: 18px 22px;
		max-width: 560px;
	}
	.lm-contacts__multi {
		display: flex;
		align-items: flex-end;
		gap: 6px;
	}
	.lm-contacts__multi > :global(:first-child) {
		flex: 1;
	}
	.lm-link-btn {
		align-self: flex-start;
		border: 0;
		padding: 0;
		background: transparent;
		color: var(--ca-brand-text);
		font: inherit;
		font-size: 13px;
		cursor: pointer;
	}
	.lm-contacts__actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
	}
	.lm-contacts__empty {
		flex: 1;
		display: grid;
		place-items: center;
	}
	.lm-contacts__list--full {
		flex: 1;
		width: auto;
		border-right: 0;
	}
	.lm-pane[hidden] {
		display: none;
	}
	@media (max-width: 980px) {
		.lm-contacts__groups {
			display: none;
		}
		.lm-contacts__list {
			width: 280px;
		}
	}
</style>
