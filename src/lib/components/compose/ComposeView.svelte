<script lang="ts">
	import { Button, DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuItem, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, IconButton, Select, Spinner, Tooltip } from "@codeatlasdev/ui";
	import { Ellipsis, File as FileIcon, Minus, Paperclip, Send, Trash2, X } from "@lucide/svelte";
	import { isDesktop, uploadFromDisk } from "../../api";
	import { app } from "../../state/app.svelte";
	import { compose } from "../../state/compose.svelte";
	import { t } from "../../i18n/index.svelte";
	import { size } from "../../format";
	import type { Priority } from "../../types";
	import RecipientField from "./RecipientField.svelte";
	import Editor from "./Editor.svelte";
	import { toast } from "@codeatlasdev/ui";

	let fileInput: HTMLInputElement | undefined = $state();
	let editor: Editor | undefined = $state();

	// Actions (send, save, close) take the last keystrokes from the editor.
	$effect(() => {
		compose.flushEditor = () => editor?.flush();
		return () => (compose.flushEditor = null);
	});
	let dragging = $state(false);
	let dragDepth = 0;

	const d = $derived(compose.draft);
	const title = $derived.by(() => {
		switch (d?.mode) {
			case "reply":
			case "replyAll":
				return t("compose.title.reply");
			case "forward":
			case "forwardAttachment":
				return t("compose.title.forward");
			case "draft":
				return t("compose.title.draft");
			default:
				return t("compose.newMessage");
		}
	});
	const identityOptions = $derived(app.identities.map((i) => ({ value: i.id, label: i.name ? `${i.name} <${i.email}>` : i.email })));
	const status = $derived(
		compose.saving ? t("compose.saving") : compose.savedAt ? t("compose.savedAt", { time: new Date(compose.savedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) }) : "",
	);

	function onKeydown(e: KeyboardEvent) {
		const ctrl = e.ctrlKey || e.metaKey;
		if (ctrl && e.key === "Enter") {
			e.preventDefault();
			void compose.send();
		} else if (ctrl && e.key.toLowerCase() === "s") {
			e.preventDefault();
			void compose.save();
		}
	}

	async function attach() {
		if (isDesktop) {
			try {
				const ups = await uploadFromDisk();
				for (const u of ups) compose.attach(u);
			} catch (e) {
				toast.error(String((e as Error).message ?? e));
			}
		} else fileInput?.click();
	}

	function onDragEnter(e: DragEvent) {
		if (!e.dataTransfer?.types.includes("Files")) return;
		dragDepth++;
		dragging = true;
	}

	function onDragLeave() {
		dragDepth = Math.max(0, dragDepth - 1);
		if (!dragDepth) dragging = false;
	}

	function onDrop(e: DragEvent) {
		dragDepth = 0;
		dragging = false;
		const files = Array.from(e.dataTransfer?.files ?? []);
		if (!files.length) return;
		e.preventDefault();
		void compose.addFiles(files);
	}
</script>

<svelte:window onkeydown={onKeydown} />

{#if d}
	<div
		class="lm-pane lm-compose"
		role="region"
		aria-label={title}
		ondragenter={onDragEnter}
		ondragleave={onDragLeave}
		ondragover={(e) => e.dataTransfer?.types.includes("Files") && e.preventDefault()}
		ondrop={onDrop}
	>
		<div class="lm-bar">
			<span class="lm-bar__title">{title}</span>
			<span class="lm-hint lm-compose__status" aria-live="polite">{status}</span>
			<span class="lm-spacer"></span>
			<Tooltip content={t("compose.minimize")}>
				<IconButton variant="text" size="sm" aria-label={t("compose.minimize")} onclick={() => compose.hide()}><Minus size={16} /></IconButton>
			</Tooltip>
			<Tooltip content={t("compose.close")}>
				<IconButton variant="text" size="sm" aria-label={t("compose.close")} onclick={() => compose.closeAsking()}><X size={16} /></IconButton>
			</Tooltip>
		</div>

		<div class="lm-compose__fields">
			{#if app.identities.length > 1}
				<div class="lm-compose__row">
					<span class="lm-compose__label">{t("compose.from")}</span>
					<div class="lm-compose__value">
						<Select size="sm" value={d.identityId ?? app.defaultIdentity?.id ?? ""} options={identityOptions} aria-label={t("compose.from")} onchange={(v) => compose.setIdentity(v)} />
					</div>
				</div>
			{/if}
			<div class="lm-compose__to">
				<div class="lm-compose__grow"><RecipientField label={t("compose.to")} bind:addresses={d.to} autofocus={d.to.length === 0} /></div>
				<div class="lm-compose__toggles">
					{#if !compose.showCc}<button type="button" class="lm-link" onclick={() => (compose.showCc = true)}>{t("compose.cc")}</button>{/if}
					{#if !compose.showBcc}<button type="button" class="lm-link" onclick={() => (compose.showBcc = true)}>{t("compose.bcc")}</button>{/if}
				</div>
			</div>
			{#if compose.showCc}<RecipientField label={t("compose.cc")} bind:addresses={d.cc} />{/if}
			{#if compose.showBcc}<RecipientField label={t("compose.bcc")} bind:addresses={d.bcc} />{/if}
			<div class="lm-compose__row">
				<label class="lm-compose__label" for="lm-subject">{t("compose.subject")}</label>
				<input id="lm-subject" class="lm-compose__subject" bind:value={d.subject} autocomplete="off" />
			</div>
		</div>

		{#if d.attachments.length || compose.uploading.length}
			<ul class="lm-compose__files" aria-label={t("message.attachments")}>
				{#each d.attachments as a, i (i)}
					<li class="lm-attach">
						<FileIcon size={15} />
						<span class="lm-ellipsis" title={a.filename}>{a.filename}</span>
						<span class="lm-hint">{size(a.size)}</span>
						<button type="button" class="lm-attach__x" aria-label={t("compose.removeAttachment", { name: a.filename })} onclick={() => compose.removeAttachment(a)}><X size={13} /></button>
					</li>
				{/each}
				{#each compose.uploading as u (u.id)}
					<li class="lm-attach lm-attach--busy">
						<Spinner size={13} />
						<span class="lm-ellipsis">{u.name}</span>
						<span class="lm-hint">{size(u.size)}</span>
					</li>
				{/each}
			</ul>
		{/if}

		<div class="lm-compose__body">
			{#if d.html !== null}
				<Editor bind:this={editor} html={d.html ?? ""} version={compose.bodyVersion} onchange={(h) => d && (d.html = h)} onkeydown={onKeydown} placeholder={t("compose.bodyPlaceholder")} />
			{:else}
				<textarea class="lm-compose__plain" bind:value={d.text} spellcheck="true" aria-label={t("editor.body")} placeholder={t("compose.bodyPlaceholder")}></textarea>
			{/if}
		</div>

		<div class="lm-compose__actions">
			<Button variant="filled" loading={compose.sending} onclick={() => compose.send()}>
				{#snippet leading()}<Send size={15} />{/snippet}
				{t("compose.send")}
			</Button>
			<Tooltip content={t("compose.attach")}>
				<IconButton variant="outlined" aria-label={t("compose.attach")} onclick={attach}><Paperclip size={16} /></IconButton>
			</Tooltip>
			<Button variant="ghost" loading={compose.saving} onclick={() => compose.save()}>{t("compose.saveDraft")}</Button>
			<span class="lm-spacer"></span>
			<DropdownMenu side="top" align="end">
				{#snippet triggerChild(props)}
					<button {...props} type="button" class="lm-ghost-btn" aria-label={t("compose.options")}><Ellipsis size={16} /> <span class="lm-hide-phone">{t("compose.options")}</span></button>
				{/snippet}
				<DropdownMenuLabel>{t("compose.priority")}</DropdownMenuLabel>
				<DropdownMenuRadioGroup value={d.priority} onValueChange={(v: string) => d && (d.priority = v as Priority)}>
					<DropdownMenuRadioItem value="high">{t("priority.high")}</DropdownMenuRadioItem>
					<DropdownMenuRadioItem value="normal">{t("priority.normal")}</DropdownMenuRadioItem>
					<DropdownMenuRadioItem value="low">{t("priority.low")}</DropdownMenuRadioItem>
				</DropdownMenuRadioGroup>
				<DropdownMenuSeparator />
				<DropdownMenuCheckboxItem checked={d.requestReceipt} onCheckedChange={(c) => d && (d.requestReceipt = c)}>{t("compose.requestReceipt")}</DropdownMenuCheckboxItem>
				<DropdownMenuCheckboxItem checked={d.html !== null} onCheckedChange={(c) => compose.setHtml(c)}>{t("compose.richText")}</DropdownMenuCheckboxItem>
			</DropdownMenu>
			<Tooltip content={t("compose.discard.action")}>
				<IconButton variant="text" aria-label={t("compose.discard.action")} onclick={() => compose.discard()}><Trash2 size={16} /></IconButton>
			</Tooltip>
		</div>

		<input
			bind:this={fileInput}
			type="file"
			multiple
			hidden
			onchange={(e) => {
				const input = e.currentTarget as HTMLInputElement;
				void compose.addFiles(Array.from(input.files ?? []));
				input.value = "";
			}}
		/>

		{#if dragging}
			<div class="lm-compose__drop" aria-hidden="true">{t("compose.dropFiles")}</div>
		{/if}
	</div>
{:else if compose.loading}
	<div class="lm-compose__loading"><Spinner size={24} label={t("app.loading")} /></div>
{/if}

<style>
	.lm-compose {
		position: relative;
		flex: 1;
		min-height: 0;
	}
	.lm-compose__status {
		margin-left: 8px;
	}
	.lm-compose__fields {
		padding: 0 16px;
		flex-shrink: 0;
	}
	.lm-compose__row {
		display: flex;
		align-items: center;
		gap: 8px;
		min-height: 38px;
		border-bottom: 1px solid var(--ca-outline-subtle);
	}
	.lm-compose__label {
		flex: 0 0 64px;
		color: var(--ca-text-secondary);
		font-size: 13px;
	}
	.lm-compose__value {
		flex: 1;
		max-width: 480px;
	}
	.lm-compose__to {
		display: flex;
		align-items: flex-start;
	}
	.lm-compose__grow {
		flex: 1;
		min-width: 0;
	}
	.lm-compose__toggles {
		display: flex;
		gap: 10px;
		padding: 10px 0 0 10px;
		border-bottom: 1px solid var(--ca-outline-subtle);
		align-self: stretch;
	}
	.lm-link {
		border: 0;
		padding: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		font: inherit;
		font-size: 13px;
		cursor: pointer;
	}
	.lm-link:hover {
		color: var(--ca-text-primary);
		text-decoration: underline;
	}
	.lm-compose__subject {
		flex: 1;
		min-width: 0;
		height: 36px;
		border: 0;
		outline: none;
		background: transparent;
		color: var(--ca-text-primary);
		font: inherit;
		font-size: 15px;
		font-weight: 600;
	}
	.lm-compose__files {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		margin: 0;
		padding: 8px 16px 0;
		list-style: none;
	}
	.lm-attach {
		display: flex;
		align-items: center;
		gap: 6px;
		max-width: 300px;
		height: 30px;
		padding: 0 4px 0 8px;
		border: 1px solid var(--ca-outline-default);
		background: var(--ca-surface-sunken);
		font-size: 13px;
	}
	.lm-attach--busy {
		opacity: 0.7;
		padding-right: 10px;
	}
	.lm-attach__x {
		display: grid;
		place-items: center;
		width: 22px;
		height: 22px;
		border: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		cursor: pointer;
	}
	.lm-attach__x:hover {
		color: var(--lm-danger);
	}
	.lm-compose__body {
		flex: 1;
		min-height: 0;
		display: flex;
		flex-direction: column;
		padding: 10px 16px;
	}
	.lm-compose__plain {
		flex: 1;
		min-height: 240px;
		resize: none;
		padding: 12px 14px;
		border: 1px solid var(--ca-outline-default);
		background: var(--ca-surface-base);
		color: var(--ca-text-primary);
		font-family: var(--ca-font-mono);
		font-size: 13px;
		line-height: 1.55;
		outline: none;
	}
	.lm-compose__actions {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 16px 12px;
		flex-shrink: 0;
	}
	.lm-compose__drop {
		position: absolute;
		inset: 0;
		z-index: 10;
		display: grid;
		place-items: center;
		background: color-mix(in oklab, var(--lm-signal) 18%, transparent);
		border: 2px dashed var(--ca-outline-strong);
		font-size: 18px;
		font-weight: 700;
		pointer-events: none;
	}
	.lm-compose__loading {
		flex: 1;
		display: grid;
		place-items: center;
	}
	@media (max-width: 640px) {
		.lm-hide-phone {
			display: none;
		}
	}
</style>
