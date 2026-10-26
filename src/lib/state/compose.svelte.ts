/**
 * The message being written: opening (new, reply, forward, draft), the
 * signature of the chosen identity, attachments, autosave and sending.
 */
import { toast } from "@codeatlasdev/ui";
import { call, upload as uploadFiles } from "../api";
import { errorText, i18n, t } from "../i18n/index.svelte";
import { escapeHtml, htmlToText } from "../format";
import type { Address, ComposeMode, Draft, DraftAttachment, Identity, MessageRef, SendResult } from "../types";
import { app } from "./app.svelte";
import { choose, confirm } from "./dialogs.svelte";
import { keyOf, mail } from "./mail.svelte";

const SIGNATURE_CLASS = "lm-signature";
const TEXT_SIGNATURE = "\n-- \n";

export interface Preset {
	to?: Address[];
	cc?: Address[];
	bcc?: Address[];
	subject?: string;
	body?: string;
}

function emptyDraft(): Draft {
	return {
		mode: "new",
		identityId: null,
		from: null,
		to: [],
		cc: [],
		bcc: [],
		replyTo: [],
		subject: "",
		html: "",
		text: "",
		attachments: [],
		source: null,
		inReplyTo: null,
		references: [],
		priority: "normal",
		requestReceipt: false,
		draftRef: null,
		messageId: null,
	};
}

function signatureHtml(identity: Identity | undefined): string {
	const sig = identity?.signature?.trim();
	if (!sig) return "";
	return `<div class="${SIGNATURE_CLASS}">-- <br>${sig}</div>`;
}

function signatureText(identity: Identity | undefined): string {
	const sig = identity?.signature?.trim();
	return sig ? `${TEXT_SIGNATURE}${htmlToText(sig)}` : "";
}

export interface PendingUpload {
	id: number;
	name: string;
	size: number;
}

class ComposeState {
	draft = $state<Draft | null>(null);
	/** Shown (false: kept aside while the user reads other mail). */
	visible = $state(false);
	loading = $state(false);
	saving = $state(false);
	sending = $state(false);
	savedAt = $state<number | null>(null);
	uploading = $state<PendingUpload[]>([]);
	showCc = $state(false);
	showBcc = $state(false);
	/** Bumped when the body is replaced from outside, so the editor reloads it. */
	bodyVersion = $state(0);
	private savedSnapshot = "";
	private autosaveTimer: ReturnType<typeof setInterval> | undefined;
	private uploadSeq = 0;

	/** A message is being written (shown or kept aside). */
	get active() {
		return this.draft !== null;
	}

	get html() {
		return this.draft?.html !== null && this.draft?.html !== undefined;
	}

	get identity(): Identity | undefined {
		const d = this.draft;
		return app.identities.find((i) => i.id === d?.identityId) ?? app.defaultIdentity;
	}

	private snapshot() {
		return JSON.stringify(this.draft);
	}

	get dirty() {
		return this.draft !== null && this.snapshot() !== this.savedSnapshot;
	}

	async open(mode: ComposeMode = "new", source: MessageRef | null = null, preset: Preset = {}) {
		if (this.draft && this.dirty && !(await this.closeAsking())) return;
		this.loading = true;
		this.visible = true;
		app.view = "mail";
		try {
			// Remote images of the original are quoted only when the reader
			// allowed them; the user's own drafts keep theirs.
			const remoteImages =
				mode === "draft" ||
				app.settings.showImages === "always" ||
				(!!source && mail.open === keyOf(source.folder, source.uid) && mail.allowRemote);
			const style = {
				locale: i18n.lang,
				tzOffsetMinutes: -new Date().getTimezoneOffset(),
				replyOnTop: app.settings.replyOnTop,
				remoteImages,
			};
			const d = await call<Draft>("compose.prepare", { mode, source, html: app.settings.composeHtml, style });
			const draft = { ...emptyDraft(), ...d };
			if (preset.to) draft.to = preset.to;
			if (preset.cc) draft.cc = preset.cc;
			if (preset.bcc) draft.bcc = preset.bcc;
			if (preset.subject) draft.subject = preset.subject;
			if (mode !== "draft" && mode !== "editAsNew") this.insertSignature(draft, preset.body);
			else if (preset.body) draft.text = preset.body;
			this.draft = draft;
			this.showCc = draft.cc.length > 0;
			this.showBcc = draft.bcc.length > 0;
			this.savedSnapshot = mode === "draft" ? this.snapshot() : "";
			this.savedAt = null;
			this.bodyVersion++;
			this.startAutosave();
		} catch (e) {
			toast.error(errorText(e));
			this.visible = false;
		} finally {
			this.loading = false;
		}
	}

	private insertSignature(draft: Draft, body?: string) {
		const identity = app.identities.find((i) => i.id === draft.identityId) ?? app.defaultIdentity;
		if (draft.html !== null) {
			const sig = signatureHtml(identity);
			const intro = body ? `<p>${escapeHtml(body).replace(/\n/g, "<br>")}</p>` : "<p><br></p>";
			const rest = (draft.html ?? "").replace(/^(<p><br><\/p>)+/, "");
			const quoted = rest.trim().length > 0;
			if (quoted && app.settings.replyOnTop) draft.html = `${intro}${sig ? `<p><br></p>${sig}` : ""}<p><br></p>${rest}`;
			else if (quoted) draft.html = `${rest}${intro}${sig}`;
			else draft.html = `${intro}${sig ? `<p><br></p>${sig}` : ""}`;
		} else {
			const sig = signatureText(identity);
			const intro = body ?? "";
			const rest = draft.text.replace(/^\n+/, "");
			if (rest && app.settings.replyOnTop) draft.text = `${intro}\n${sig}\n\n${rest}`;
			else if (rest) draft.text = `${rest}\n${intro}${sig}`;
			else draft.text = `${intro}\n${sig}`;
		}
	}

	/** Set by the editor: hands over the last keystrokes before an action. */
	flushEditor: (() => void) | null = null;
	private savePromise: Promise<boolean> | null = null;

	private flush() {
		this.flushEditor?.();
	}

	/** Switches the sender, swapping the signature in the body. */
	setIdentity(id: string) {
		this.flush();
		const d = this.draft;
		if (!d) return;
		const old = this.identity;
		d.identityId = id;
		const next = this.identity;
		d.from = next ? { name: next.name, email: next.email } : d.from;
		if (d.html !== null) {
			const doc = new DOMParser().parseFromString(`<body>${d.html}</body>`, "text/html");
			// Our signature, never one inside the quoted message.
			const block = Array.from(doc.querySelectorAll(`.${SIGNATURE_CLASS}`)).find((el) => !el.closest("blockquote"));
			const sig = signatureHtml(next);
			if (block) {
				if (sig) block.outerHTML = sig;
				else block.remove();
			} else if (sig) {
				const quote = doc.querySelector("blockquote, .lm-quote-header, .lm-forward");
				if (quote && app.settings.replyOnTop) quote.insertAdjacentHTML("beforebegin", `${sig}<p><br></p>`);
				else doc.body.insertAdjacentHTML(app.settings.replyOnTop ? "afterbegin" : "beforeend", `<p><br></p>${sig}`);
			}
			d.html = doc.body.innerHTML;
		} else {
			const oldSig = signatureText(old);
			const sig = signatureText(next);
			d.text = oldSig && d.text.includes(oldSig) ? d.text.replace(oldSig, sig) : d.text + sig;
		}
		this.bodyVersion++;
	}

	/** Switches between HTML and plain text, converting the body. */
	setHtml(on: boolean) {
		this.flush();
		const d = this.draft;
		if (!d || on === (d.html !== null)) return;
		if (on) {
			d.html = escapeHtml(d.text).replace(/\n/g, "<br>");
			d.text = "";
		} else {
			d.text = htmlToText(d.html ?? "");
			d.html = null;
		}
		this.bodyVersion++;
	}

	async addFiles(files: File[]) {
		const d = this.draft;
		if (!d || !files.length) return;
		const entries = files.map((f) => ({ id: ++this.uploadSeq, name: f.name, size: f.size }));
		this.uploading = [...this.uploading, ...entries];
		try {
			const uploads = await uploadFiles(files);
			// The message may have been closed or replaced while uploading.
			if (this.draft === d) for (const u of uploads) this.attach(u);
			else for (const u of uploads) void call("uploads.delete", { id: u.id }).catch(() => {});
		} catch (e) {
			toast.error(errorText(e));
		} finally {
			const ids = new Set(entries.map((e) => e.id));
			this.uploading = this.uploading.filter((u) => !ids.has(u.id));
		}
	}

	attach(u: { id: string; filename: string; contentType: string; size: number }) {
		this.draft?.attachments.push({ source: "upload", id: u.id, filename: u.filename, contentType: u.contentType, size: u.size });
	}

	removeAttachment(a: DraftAttachment) {
		const d = this.draft;
		if (!d) return;
		d.attachments = d.attachments.filter((x) => x !== a);
		if (a.source === "upload") void call("uploads.delete", { id: a.id }).catch(() => {});
	}

	private startAutosave() {
		clearInterval(this.autosaveTimer);
		const secs = app.settings.autosaveSeconds;
		if (secs > 0) {
			this.autosaveTimer = setInterval(() => {
				if (this.dirty && !this.saving && !this.sending && this.hasContent()) void this.save(true);
			}, secs * 1000);
		}
	}

	private hasContent() {
		const d = this.draft;
		if (!d) return false;
		const body = d.html !== null ? htmlToText(d.html) : d.text;
		return !!(d.subject.trim() || d.to.length || d.attachments.length || body.replace(/--\s*$/m, "").trim().length > 2);
	}

	/** Waits for a save in progress (autosave), so its draft is known. */
	private async settle() {
		while (this.savePromise) await this.savePromise.catch(() => false);
	}

	/** Saves the draft in Drafts. True when it was saved. */
	async save(quiet = false): Promise<boolean> {
		this.flush();
		await this.settle();
		const d = this.draft;
		if (!d) return false;
		this.saving = true;
		const snapshot = this.snapshot();
		const run = (async () => {
			try {
				const saved = await call<Draft>("compose.saveDraft", { draft: $state.snapshot(d) });
				d.draftRef = saved.draftRef;
				d.messageId = saved.messageId;
				if (this.draft === d) {
					// Edits made while saving stay unsaved.
					this.savedSnapshot =
						snapshot === this.snapshot() ? this.snapshot() : JSON.stringify({ ...JSON.parse(snapshot), draftRef: saved.draftRef, messageId: saved.messageId });
					this.savedAt = Date.now();
				}
				if (!quiet) toast.success(t("compose.saved"));
				const drafts = mail.folderByRole("drafts");
				if (drafts) void mail.refreshCounts([drafts.path]);
				return true;
			} catch (e) {
				if (!quiet) toast.error(errorText(e));
				return false;
			}
		})();
		this.savePromise = run;
		try {
			return await run;
		} finally {
			if (this.savePromise === run) this.savePromise = null;
			this.saving = false;
		}
	}

	private mentionsAttachment(d: Draft) {
		const body = d.html !== null ? htmlToText(d.html.split("<blockquote")[0]) : d.text.split(/\n>/)[0];
		return /\b(anexo|anexa|anexad[oa]s?|em anexo|segue anexo|attach(ed|ment|ments)?|enclosed)\b/i.test(`${d.subject}\n${body}`);
	}

	async send() {
		this.flush();
		const d = this.draft;
		if (!d || this.sending) return;
		if (!d.to.length && !d.cc.length && !d.bcc.length) {
			toast.error(t("compose.noRecipients"));
			return;
		}
		if (!d.subject.trim() && !(await confirm({ title: t("compose.noSubject.title"), description: t("compose.noSubject.body"), confirmLabel: t("compose.sendAnyway") }))) return;
		if (!d.attachments.length && this.mentionsAttachment(d) && !(await confirm({ title: t("compose.noAttachment.title"), description: t("compose.noAttachment.body"), confirmLabel: t("compose.sendAnyway") }))) return;
		if (this.uploading.length) {
			toast.info(t("compose.waitUploads"));
			return;
		}
		this.sending = true;
		clearInterval(this.autosaveTimer);
		try {
			// An autosave in flight decides which saved draft send removes.
			await this.settle();
			const res = await call<SendResult>("compose.send", { draft: $state.snapshot(d), saveCopy: app.settings.saveSent });
			for (const w of res.warnings ?? []) toast.warning(t(`compose.warning.${w}`), { duration: 10_000 });
			toast.success(t("compose.sent"));
			this.reset(false);
			void mail.refreshCounts();
			if (mail.current?.role === "sent" || mail.current?.role === "drafts") mail.refresh();
			void app.loadIdentities();
		} catch (e) {
			toast.error(errorText(e), { duration: 10_000 });
			this.startAutosave();
		} finally {
			this.sending = false;
		}
	}

	/** Closes the editor; asks to keep the draft when it has unsaved changes. */
	async closeAsking(): Promise<boolean> {
		this.flush();
		await this.settle();
		const d = this.draft;
		if (!d) return true;
		if (this.dirty && this.hasContent()) {
			const answer = await choose({
				title: t("compose.keep.title"),
				description: t("compose.keep.body"),
				choices: [
					{ value: "discard", label: t("compose.keep.discard"), variant: "ghost" },
					{ value: "edit", label: t("compose.keep.edit"), variant: "outlined" },
					{ value: "save", label: t("compose.keep.save"), variant: "filled" },
				],
			});
			if (answer === null || answer === "edit") {
				this.show();
				return false;
			}
			if (answer === "save") {
				if (!(await this.save())) {
					// Not saved: keep the message open rather than lose it.
					this.show();
					return false;
				}
			} else {
				await this.discard(false);
				return true;
			}
		}
		this.reset();
		return true;
	}

	async discard(ask = true) {
		this.flush();
		if (!this.draft) return;
		if (ask && this.hasContent() && !(await confirm({ title: t("compose.discard.title"), description: t("compose.discard.body"), confirmLabel: t("compose.discard.confirm"), destructive: true }))) return;
		clearInterval(this.autosaveTimer);
		await this.settle();
		const d = this.draft;
		if (!d) return;
		if (d.draftRef) {
			try {
				await call("compose.discardDraft", { folder: d.draftRef.folder, uid: d.draftRef.uid });
				mail.refresh();
			} catch {
				// The draft stays in Drafts; nothing is lost.
			}
		}
		this.reset();
	}

	/** Forgets the message. Its uploaded files go too: a saved draft keeps
	 * its own copies, and a sent message no longer needs them. */
	private reset(dropUploads = true) {
		clearInterval(this.autosaveTimer);
		if (dropUploads) for (const a of this.draft?.attachments ?? []) if (a.source === "upload") void call("uploads.delete", { id: a.id }).catch(() => {});
		this.draft = null;
		this.visible = false;
		this.savedSnapshot = "";
		this.savedAt = null;
		this.uploading = [];
	}

	hide() {
		this.flush();
		this.visible = false;
	}

	show() {
		if (this.draft) {
			this.visible = true;
			app.view = "mail";
		}
	}

	/** Opens the editor for a `mailto:` link. */
	async mailto(href: string) {
		let url: URL;
		try {
			url = new URL(href);
		} catch {
			return;
		}
		const params = url.searchParams;
		const decode = (s: string) => {
			try {
				return decodeURIComponent(s);
			} catch {
				return s;
			}
		};
		// searchParams values are decoded already; the path is not.
		const list = (v: string | null) =>
			(v ?? "")
				.split(",")
				.map((s) => s.trim())
				.filter(Boolean)
				.map((email) => ({ email, name: null }));
		await this.open("new", null, {
			to: list(decode(url.pathname) || null),
			cc: list(params.get("cc")),
			bcc: list(params.get("bcc")),
			subject: params.get("subject") ?? undefined,
			body: params.get("body") ?? undefined,
		});
	}
}

export const compose = new ComposeState();
