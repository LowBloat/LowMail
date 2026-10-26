<script lang="ts">
	import { Avatar, Button, Dialog, DropdownMenu, DropdownMenuItem, DropdownMenuSeparator, EmptyState, IconButton, Lightbox, Skeleton, Tooltip, colorScheme, toast } from "@codeatlasdev/ui";
	import {
		Archive,
		ArrowLeft,
		CircleAlert,
		Download,
		Ellipsis,
		ExternalLink,
		File,
		FileArchive,
		FileImage,
		FileSpreadsheet,
		FileText,
		Flag,
		Forward,
		Image,
		Mail,
		MailCheck,
		MailOpen,
		MessagesSquare,
		OctagonAlert,
		Reply,
		ReplyAll,
		Trash2,
	} from "@lucide/svelte";
	import { call, isDesktop, openAttachment, openExternal, saveAttachment, saveMessage } from "../../api";
	import { app } from "../../state/app.svelte";
	import { compose } from "../../state/compose.svelte";
	import { keyOf, mail } from "../../state/mail.svelte";
	import { errorText, t } from "../../i18n/index.svelte";
	import { displayName, formatAddress, fullDate, initials, listDate, size } from "../../format";
	import type { Address, AttachmentInfo, MessageSummary } from "../../types";
	import MessageFrame from "./MessageFrame.svelte";
	import { printMessage } from "../../print";

	let { back = false }: { back?: boolean } = $props();

	let details = $state(false);
	let showHeaders = $state(false);
	let source = $state<string | null>(null);
	let receiptDismissed = $state(false);

	const m = $derived(mail.message);
	const ref = $derived(m ? { folder: m.folder, uid: m.uid } : null);
	const files = $derived(m?.attachments.filter((a) => !a.inline) ?? []);
	const inFolder = $derived(m ? mail.folders.find((f) => f.path === m.folder) : undefined);
	const totalFiles = $derived(files.reduce((s, a) => s + a.size, 0));
	const wantsReceipt = $derived(
		!!m?.receiptTo && !receiptDismissed && app.settings.readReceipts === "ask" && !m.flags.keywords?.some((k) => k.toLowerCase() === "$mdnsent") && inFolder?.role !== "sent",
	);
	const unsubscribe = $derived.by(() => {
		const raw = m?.unsubscribe ?? "";
		const links = [...raw.matchAll(/<([^>]+)>/g)].map((x) => x[1]);
		return links.find((l) => /^https?:/i.test(l)) ?? links.find((l) => /^mailto:/i.test(l)) ?? null;
	});

	$effect(() => {
		void mail.open;
		details = false;
		showHeaders = false;
		receiptDismissed = false;
	});

	// The conversation of the open message, loaded after the message itself.
	let thread = $state<MessageSummary[]>([]);
	let threadOpen = $state(false);
	let threadKey = "";
	$effect(() => {
		const msg = m;
		const key = msg ? keyOf(msg.folder, msg.uid) : "";
		if (!msg || key === threadKey) return;
		const keepOpen = thread.some((t) => keyOf(t.folder, t.uid) === key);
		threadKey = key;
		if (!keepOpen) {
			thread = [];
			threadOpen = false;
		}
		void call<MessageSummary[]>("messages.thread", { folder: msg.folder, uid: msg.uid })
			.then((list) => {
				if (threadKey === key) thread = list;
			})
			.catch(() => {});
	});

	// Previews of image attachments, loaded on demand.
	const images = $derived(files.filter((a) => a.contentType.startsWith("image/") && a.contentType !== "image/svg+xml" && a.size <= 8 * 1024 * 1024));
	let previews = $state<Record<string, string>>({});
	let lightbox = $state(-1);
	$effect(() => {
		const msg = m;
		previews = {};
		if (!msg) return;
		for (const a of images) {
			void call<{ url: string }>("messages.attachmentData", { folder: msg.folder, uid: msg.uid, part: a.part })
				.then((r) => {
					if (mail.message === msg) previews = { ...previews, [a.part]: r.url };
				})
				.catch(() => {});
		}
	});
	const lightboxItems = $derived(images.filter((a) => previews[a.part]).map((a) => ({ src: previews[a.part], alt: a.filename, caption: a.filename })));

	function openFromThread(s: MessageSummary) {
		void mail.openMessage(keyOf(s.folder, s.uid));
	}

	function fileIcon(a: AttachmentInfo) {
		const ct = a.contentType;
		if (ct.startsWith("image/")) return FileImage;
		if (ct === "application/pdf" || ct.startsWith("text/") || ct.includes("word") || ct.includes("opendocument.text")) return FileText;
		if (ct.includes("zip") || ct.includes("compressed") || ct.includes("tar") || ct.includes("rar")) return FileArchive;
		if (ct.includes("sheet") || ct.includes("excel") || ct === "text/csv") return FileSpreadsheet;
		if (ct === "message/rfc822") return Mail;
		return File;
	}

	async function viewSource() {
		if (!ref) return;
		try {
			const res = await call<{ source: string }>("messages.source", ref);
			source = res.source;
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	function people(list: Address[]) {
		return list.map((a) => (app.isMine(a.email) ? t("message.me") : displayName(a))).join(", ");
	}

	function writeTo(a: Address) {
		void compose.open("new", null, { to: [a] });
	}

	async function addContact(a: Address) {
		try {
			await call("contacts.save", { contact: { name: a.name ?? "", emails: [a.email] } });
			toast.success(t("contacts.added", { name: displayName(a) }));
		} catch (e) {
			toast.error(errorText(e));
		}
	}

	function searchFrom(a: Address) {
		mail.search({ ...$state.snapshot(mail.filter), text: a.email, scope: "from" }, true);
	}

	async function unsubscribeNow() {
		if (!unsubscribe) return;
		if (/^mailto:/i.test(unsubscribe)) void compose.mailto(unsubscribe);
		else void openExternal(unsubscribe);
	}
</script>

{#snippet addressMenu(a: Address)}
	<DropdownMenu>
		{#snippet triggerChild(props)}
			<button {...props} type="button" class="lm-addr" title={a.email}>{displayName(a)}</button>
		{/snippet}
		<DropdownMenuItem onclick={() => writeTo(a)}>{t("message.writeTo")}</DropdownMenuItem>
		<DropdownMenuItem onclick={() => addContact(a)}>{t("message.addContact")}</DropdownMenuItem>
		<DropdownMenuItem onclick={() => searchFrom(a)}>{t("message.searchFrom")}</DropdownMenuItem>
		<DropdownMenuItem onclick={() => navigator.clipboard.writeText(a.email).then(() => toast.success(t("message.copied")))}>{t("message.copyAddress")}</DropdownMenuItem>
	</DropdownMenu>
{/snippet}

<div class="lm-pane lm-reader">
	{#if !mail.open}
		<div class="lm-reader__empty">
			<EmptyState icon={Mail} title={mail.selected.size ? t("reader.selected", { n: mail.selected.size }) : t("reader.none")} description={mail.selected.size ? "" : t("reader.noneHint")} />
		</div>
	{:else if mail.messageError && !m}
		<div class="lm-reader__empty">
			<EmptyState icon={CircleAlert} title={t("reader.error")} description={mail.messageError}>
				{#snippet actions()}<Button variant="outlined" size="sm" onclick={() => mail.open && mail.openMessage(mail.open)}>{t("action.retry")}</Button>{/snippet}
			</EmptyState>
		</div>
	{:else if !m}
		<div class="lm-reader__loading" aria-busy="true">
			<Skeleton variant="text" width="60%" height={24} />
			<Skeleton variant="text" rows={2} />
			<Skeleton variant="rectangular" height={220} />
		</div>
	{:else}
		<div class="lm-bar lm-reader__bar">
			{#if back}
				<IconButton variant="text" size="sm" aria-label={t("action.back")} onclick={() => mail.closeMessage()}><ArrowLeft size={17} /></IconButton>
			{/if}
			<Button variant="ghost" size="sm" onclick={() => ref && compose.open("reply", ref)}>
				{#snippet leading()}<Reply size={15} />{/snippet}
				<span class="lm-hide-phone">{t("action.reply")}</span>
			</Button>
			<Button variant="ghost" size="sm" onclick={() => ref && compose.open("replyAll", ref)}>
				{#snippet leading()}<ReplyAll size={15} />{/snippet}
				<span class="lm-hide-narrow">{t("action.replyAll")}</span>
			</Button>
			<DropdownMenu>
				{#snippet triggerChild(props)}
					<button {...props} type="button" class="lm-ghost-btn">
						<Forward size={15} />
						<span class="lm-hide-narrow">{t("action.forward")}</span>
					</button>
				{/snippet}
				<DropdownMenuItem onclick={() => ref && compose.open("forward", ref)}>{t("action.forwardInline")}</DropdownMenuItem>
				<DropdownMenuItem onclick={() => ref && compose.open("forwardAttachment", ref)}>{t("action.forwardAttachment")}</DropdownMenuItem>
			</DropdownMenu>
			<span class="lm-spacer"></span>
			<Tooltip content={`${t("action.archive")} (E)`}><IconButton variant="text" size="sm" aria-label={t("action.archive")} onclick={() => mail.archive()}><Archive size={16} /></IconButton></Tooltip>
			{#if inFolder?.role === "junk"}
				<Tooltip content={t("action.notJunk")}><IconButton variant="text" size="sm" aria-label={t("action.notJunk")} onclick={() => mail.junk(false)}><MailCheck size={16} /></IconButton></Tooltip>
			{:else}
				<Tooltip content={`${t("action.junk")} (!)`}><IconButton variant="text" size="sm" aria-label={t("action.junk")} onclick={() => mail.junk(true)}><OctagonAlert size={16} /></IconButton></Tooltip>
			{/if}
			<Tooltip content={`${t("action.delete")} (Del)`}><IconButton variant="text" size="sm" aria-label={t("action.delete")} onclick={() => mail.remove()}><Trash2 size={16} /></IconButton></Tooltip>
			<DropdownMenu align="end">
				{#snippet triggerChild(props)}
					<button {...props} type="button" class="lm-reader__more" aria-label={t("action.more")} title={t("action.more")}><Ellipsis size={17} /></button>
				{/snippet}
				<DropdownMenuItem onclick={() => mail.setFlags([keyOf(m.folder, m.uid)], { seen: !m.flags.seen })}>
					{#snippet icon()}{#if m.flags.seen}<Mail size={14} />{:else}<MailOpen size={14} />{/if}{/snippet}
					{m.flags.seen ? t("action.markUnread") : t("action.markRead")}
				</DropdownMenuItem>
				<DropdownMenuItem onclick={() => mail.setFlags([keyOf(m.folder, m.uid)], { flagged: !m.flags.flagged })}>
					{#snippet icon()}<Flag size={14} />{/snippet}
					{m.flags.flagged ? t("action.unflag") : t("action.flag")}
				</DropdownMenuItem>
				<DropdownMenuItem onclick={() => window.dispatchEvent(new CustomEvent("lm:move"))}>{t("action.moveTo")}</DropdownMenuItem>
				<DropdownMenuItem onclick={() => window.dispatchEvent(new CustomEvent("lm:move", { detail: { copy: true } }))}>{t("action.copyTo")}</DropdownMenuItem>
				<DropdownMenuSeparator />
				<DropdownMenuItem onclick={() => ref && compose.open("editAsNew", ref)}>{t("action.editAsNew")}</DropdownMenuItem>
				<DropdownMenuItem onclick={() => printMessage(m)}>{t("action.print")}</DropdownMenuItem>
				<DropdownMenuItem onclick={() => ref && saveMessage(ref.folder, ref.uid)}>{t("action.downloadEml")}</DropdownMenuItem>
				<DropdownMenuItem onclick={() => (showHeaders = !showHeaders)}>{showHeaders ? t("action.hideHeaders") : t("action.showHeaders")}</DropdownMenuItem>
				<DropdownMenuItem onclick={viewSource}>{t("action.viewSource")}</DropdownMenuItem>
				<DropdownMenuSeparator />
				<DropdownMenuItem variant="destructive" onclick={() => mail.remove(undefined, true)}>{t("action.deleteForever")}</DropdownMenuItem>
			</DropdownMenu>
		</div>

		<div class="lm-scroll lm-reader__scroll">
			<header class="lm-reader__head">
				<div class="lm-reader__subject">
					<h1>{m.subject || t("message.noSubject")}</h1>
					<button type="button" class="lm-reader__flag" class:lm-reader__flag--on={m.flags.flagged} aria-label={m.flags.flagged ? t("action.unflag") : t("action.flag")} onclick={() => mail.setFlags([keyOf(m.folder, m.uid)], { flagged: !m.flags.flagged })}>
						<Flag size={17} />
					</button>
				</div>
				<div class="lm-reader__sender">
					<Avatar size="md" fallback={initials(m.from[0])} alt="" decorative />
					<div class="lm-reader__who">
						<div class="lm-reader__from">
							{#each m.from as a (a.email)}
								{@render addressMenu(a)}
								<span class="lm-hint lm-ellipsis">&lt;{a.email}&gt;</span>
							{/each}
						</div>
						<button type="button" class="lm-reader__to lm-ellipsis" onclick={() => (details = !details)} aria-expanded={details}>
							{t("message.toLine", { to: people([...m.to, ...m.cc]) || t("list.noRecipients") })} ▾
						</button>
					</div>
					<span class="lm-reader__date" title={fullDate(m.date)}>{listDate(m.date)}</span>
				</div>
				{#if details}
					<dl class="lm-reader__details">
						<dt>{t("message.from")}</dt>
						<dd>{m.from.map(formatAddress).join(", ")}</dd>
						{#if m.replyTo.length}<dt>{t("message.replyTo")}</dt><dd>{m.replyTo.map(formatAddress).join(", ")}</dd>{/if}
						<dt>{t("message.to")}</dt>
						<dd>{#each m.to as a, i (a.email)}{#if i > 0}, {/if}{@render addressMenu(a)}{/each}</dd>
						{#if m.cc.length}<dt>{t("message.cc")}</dt><dd>{#each m.cc as a, i (a.email)}{#if i > 0}, {/if}{@render addressMenu(a)}{/each}</dd>{/if}
						{#if m.bcc.length}<dt>{t("message.bcc")}</dt><dd>{m.bcc.map(formatAddress).join(", ")}</dd>{/if}
						<dt>{t("message.date")}</dt>
						<dd>{fullDate(m.date)}</dd>
						<dt>{t("message.size")}</dt>
						<dd>{size(m.size)}</dd>
						{#if m.priority !== "normal"}<dt>{t("message.priority")}</dt><dd>{t(`priority.${m.priority}`)}</dd>{/if}
					</dl>
				{/if}
			</header>

			{#if m.remoteContent && !mail.allowRemote}
				<div class="lm-banner">
					<Image size={16} />
					<span>{t("message.remoteBlocked")}</span>
					<span class="lm-spacer"></span>
					<Button variant="outlined" size="sm" onclick={() => mail.showImages()}>{t("message.showImages")}</Button>
					{#if m.from[0]}
						<Button variant="ghost" size="sm" onclick={() => mail.showImages(true)}>{t("message.alwaysFrom", { sender: m.from[0].email })}</Button>
					{/if}
				</div>
			{/if}
			{#if wantsReceipt}
				<div class="lm-banner">
					<MailCheck size={16} />
					<span>{t("message.receiptAsked", { who: displayName(m.receiptTo) })}</span>
					<span class="lm-spacer"></span>
					<Button variant="outlined" size="sm" onclick={() => mail.sendReceipt()}>{t("message.sendReceipt")}</Button>
					<Button variant="ghost" size="sm" onclick={() => (receiptDismissed = true)}>{t("message.ignore")}</Button>
				</div>
			{/if}
			{#if unsubscribe}
				<div class="lm-banner lm-banner--quiet">
					<span>{t("message.listMail")}</span>
					<span class="lm-spacer"></span>
					<Button variant="ghost" size="sm" onclick={unsubscribeNow}>{t("message.unsubscribe")}</Button>
				</div>
			{/if}

			{#if showHeaders}
				<table class="lm-headers">
					<tbody>
						{#each m.headers as [name, value], i (i)}
							<tr><th>{name}</th><td>{value}</td></tr>
						{/each}
					</tbody>
				</table>
			{/if}

			{#if thread.length > 1}
				<section class="lm-thread" aria-label={t("thread.title")}>
					<button type="button" class="lm-thread__toggle" aria-expanded={threadOpen} onclick={() => (threadOpen = !threadOpen)}>
						<MessagesSquare size={15} />
						{t("thread.count", { n: thread.length })}
						<span class="lm-hint">{threadOpen ? "▴" : "▾"}</span>
					</button>
					{#if threadOpen}
						<ol class="lm-thread__list">
							{#each thread as s (keyOf(s.folder, s.uid))}
								{@const current = s.folder === m.folder && s.uid === m.uid}
								<li>
									<button type="button" class="lm-thread__item" class:lm-thread__item--current={current} class:lm-thread__item--unread={!s.flags.seen} aria-current={current ? "true" : undefined} onclick={() => openFromThread(s)}>
										<span class="lm-thread__who lm-ellipsis">{app.isMine(s.from[0]?.email ?? "") ? t("message.me") : displayName(s.from[0])}</span>
										<span class="lm-thread__subject lm-ellipsis">{s.subject || t("message.noSubject")}</span>
										<span class="lm-hint lm-thread__date">{listDate(s.date)}</span>
									</button>
								</li>
							{/each}
						</ol>
					{/if}
				</section>
			{/if}

			{#if m.html}
				{#key `${m.folder}:${m.uid}:${mail.allowRemote}`}
					<MessageFrame html={m.html} remote={mail.allowRemote} title={m.subject || t("message.noSubject")} dark={colorScheme.isDark} />
				{/key}
			{:else}
				<p class="lm-hint lm-reader__nobody">{t("message.noBody")}</p>
			{/if}

			{#if files.length}
				<section class="lm-files" aria-label={t("message.attachments")}>
					<div class="lm-files__head">
						<span>{t("message.attachmentsCount", { n: files.length })} · {size(totalFiles)}</span>
					</div>
					{#if images.length}
						<div class="lm-thumbs">
							{#each images as a (a.part)}
								{#if previews[a.part]}
									<button type="button" class="lm-thumb" title={a.filename} onclick={() => (lightbox = lightboxItems.findIndex((i) => i.alt === a.filename))}>
										<img src={previews[a.part]} alt={a.filename} loading="lazy" />
									</button>
								{/if}
							{/each}
						</div>
					{/if}
					<ul class="lm-files__list">
						{#each files as a (a.part)}
							{@const Icon = fileIcon(a)}
							<li class="lm-file">
								<Icon size={22} />
								<div class="lm-file__info">
									<span class="lm-file__name lm-ellipsis" title={a.filename}>{a.filename}</span>
									<span class="lm-hint">{size(a.size)}</span>
								</div>
								<Tooltip content={isDesktop ? t("attachment.open") : t("attachment.preview")}>
									<IconButton variant="text" size="sm" aria-label={t("attachment.open")} onclick={() => openAttachment(m.folder, m.uid, a.part)}><ExternalLink size={15} /></IconButton>
								</Tooltip>
								<Tooltip content={t("attachment.download")}>
									<IconButton variant="text" size="sm" aria-label={t("attachment.download")} onclick={() => saveAttachment(m.folder, m.uid, a.part)}><Download size={15} /></IconButton>
								</Tooltip>
							</li>
						{/each}
					</ul>
				</section>
			{/if}
		</div>
	{/if}
</div>

{#if lightbox >= 0 && lightboxItems.length}
	<Lightbox open items={lightboxItems} index={lightbox} onclose={() => (lightbox = -1)} />
{/if}

<Dialog open={source !== null} title={t("action.viewSource")} size="xl" onOpenChange={(o) => !o && (source = null)}>
	<pre class="lm-source">{source}</pre>
</Dialog>

<style>
	.lm-reader {
		flex: 1;
		min-height: 0;
	}
	.lm-reader__empty {
		flex: 1;
		display: grid;
		place-items: center;
		padding: 24px;
	}
	.lm-reader__loading {
		display: flex;
		flex-direction: column;
		gap: 16px;
		padding: 24px;
	}
	.lm-reader__bar {
		gap: 2px;
		overflow-x: auto;
		scrollbar-width: none;
	}
	.lm-reader__more {
		width: 32px;
		height: 32px;
		display: grid;
		place-items: center;
		border: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		cursor: pointer;
	}
	.lm-reader__more:hover {
		background: var(--lm-row-hover);
		color: var(--ca-text-primary);
	}
	.lm-reader__scroll {
		background: var(--ca-surface-base);
	}
	.lm-reader__head {
		padding: 18px 22px 12px;
	}
	.lm-reader__subject {
		display: flex;
		align-items: flex-start;
		gap: 10px;
	}
	.lm-reader__subject h1 {
		flex: 1;
		margin: 0;
		font-size: 20px;
		line-height: 1.3;
		font-weight: 700;
		overflow-wrap: anywhere;
	}
	.lm-reader__flag {
		border: 0;
		background: transparent;
		padding: 4px;
		color: var(--ca-text-tertiary, var(--ca-text-secondary));
		cursor: pointer;
	}
	.lm-reader__flag--on {
		color: var(--lm-flag);
	}
	.lm-reader__flag--on :global(svg) {
		fill: currentColor;
	}
	.lm-reader__sender {
		display: flex;
		align-items: center;
		gap: 12px;
		margin-top: 14px;
	}
	.lm-reader__who {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
	}
	.lm-reader__from {
		display: flex;
		align-items: baseline;
		gap: 6px;
		min-width: 0;
		flex-wrap: wrap;
	}
	.lm-addr {
		border: 0;
		padding: 0;
		background: transparent;
		color: var(--ca-text-primary);
		font: inherit;
		font-weight: 650;
		cursor: pointer;
	}
	.lm-addr:hover {
		text-decoration: underline;
	}
	.lm-reader__to {
		text-align: left;
		border: 0;
		padding: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		font: inherit;
		font-size: 13px;
		cursor: pointer;
		max-width: 100%;
	}
	.lm-reader__date {
		flex-shrink: 0;
		color: var(--ca-text-secondary);
		font-size: 13px;
	}
	.lm-reader__details {
		display: grid;
		grid-template-columns: max-content 1fr;
		gap: 4px 14px;
		margin: 12px 0 0;
		padding: 10px 12px;
		background: var(--ca-surface-sunken);
		font-size: 13px;
	}
	.lm-reader__details dt {
		color: var(--ca-text-secondary);
	}
	.lm-reader__details dd {
		margin: 0;
		overflow-wrap: anywhere;
	}
	.lm-banner {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px;
		margin: 0 22px 10px;
		padding: 8px 12px;
		border: 1px solid var(--ca-outline-default);
		border-left: 3px solid var(--lm-signal);
		background: var(--ca-surface-sunken);
		font-size: 13px;
	}
	.lm-banner--quiet {
		border-left-color: var(--ca-outline-default);
	}
	.lm-headers {
		margin: 0 22px 12px;
		font-size: 12px;
		font-family: var(--ca-font-mono);
		border-collapse: collapse;
	}
	.lm-headers th {
		text-align: left;
		vertical-align: top;
		padding: 2px 10px 2px 0;
		white-space: nowrap;
		color: var(--ca-text-secondary);
	}
	.lm-headers td {
		overflow-wrap: anywhere;
	}
	.lm-reader__nobody {
		padding: 0 22px;
	}
	.lm-thread {
		margin: 0 22px 12px;
		border: 1px solid var(--ca-outline-subtle);
	}
	.lm-thread__toggle {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 12px;
		border: 0;
		background: var(--ca-surface-sunken);
		color: var(--ca-text-primary);
		font: inherit;
		font-size: 13px;
		font-weight: 600;
		cursor: pointer;
		text-align: left;
	}
	.lm-thread__list {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.lm-thread__item {
		width: 100%;
		display: grid;
		grid-template-columns: minmax(80px, 160px) 1fr auto;
		gap: 10px;
		align-items: center;
		padding: 7px 12px;
		border: 0;
		border-top: 1px solid var(--ca-outline-subtle);
		background: transparent;
		color: var(--ca-text-secondary);
		font: inherit;
		font-size: 13px;
		text-align: left;
		cursor: pointer;
	}
	.lm-thread__item:hover {
		background: var(--lm-row-hover);
	}
	.lm-thread__item--current {
		background: var(--lm-row-open);
		color: var(--ca-text-primary);
	}
	.lm-thread__item--unread {
		color: var(--ca-text-primary);
		font-weight: 650;
	}
	.lm-thread__date {
		white-space: nowrap;
	}
	.lm-thumbs {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		margin-bottom: 10px;
	}
	.lm-thumb {
		width: 120px;
		height: 90px;
		padding: 0;
		border: 1px solid var(--ca-outline-default);
		background: var(--ca-surface-sunken);
		cursor: zoom-in;
		overflow: hidden;
	}
	.lm-thumb img {
		width: 100%;
		height: 100%;
		object-fit: cover;
		display: block;
	}
	.lm-files {
		margin: 8px 22px 28px;
		border-top: 1px solid var(--ca-outline-subtle);
		padding-top: 12px;
	}
	.lm-files__head {
		font-size: 13px;
		font-weight: 650;
		margin-bottom: 8px;
	}
	.lm-files__list {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
		gap: 8px;
	}
	.lm-file {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 8px 6px 8px 10px;
		border: 1px solid var(--ca-outline-default);
		background: var(--ca-surface-sunken);
		color: var(--ca-text-secondary);
	}
	.lm-file__info {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
	}
	.lm-file__name {
		color: var(--ca-text-primary);
		font-weight: 600;
		font-size: 13px;
	}
	.lm-source {
		margin: 0;
		max-height: 70vh;
		overflow: auto;
		font-size: 12px;
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}
	@media (max-width: 1100px) {
		.lm-hide-narrow {
			display: none;
		}
	}
	@media (max-width: 520px) {
		.lm-hide-phone {
			display: none;
		}
		.lm-reader__head {
			padding: 14px 14px 10px;
		}
		.lm-banner,
		.lm-files {
			margin-left: 14px;
			margin-right: 14px;
		}
	}
</style>
