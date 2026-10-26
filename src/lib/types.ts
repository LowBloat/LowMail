/** Types shared with the Rust engine (crates/lowmail-core/src/model.rs). */

export type Security = "tls" | "startTls" | "none";

export interface Endpoint {
	host: string;
	port: number;
	security: Security;
	acceptInvalidCerts: boolean;
}

export interface AccountConfig {
	id: string;
	label: string;
	name: string;
	email: string;
	username: string;
	imap: Endpoint;
	smtp: Endpoint;
	sieve: Endpoint | null;
	smtpAuth: boolean;
}

export interface AccountInfo {
	id: string;
	email: string;
	name: string;
	capabilities: string[];
	filters: boolean;
}

export type FolderRole = "inbox" | "sent" | "drafts" | "trash" | "junk" | "archive";

export interface Folder {
	path: string;
	name: string;
	displayPath: string;
	parent: string | null;
	delimiter: string | null;
	depth: number;
	role: FolderRole | null;
	subscribed: boolean;
	selectable: boolean;
	canHaveChildren: boolean;
	total: number;
	unread: number;
}

export interface Address {
	name?: string | null;
	email: string;
}

export interface Flags {
	seen: boolean;
	flagged: boolean;
	answered: boolean;
	forwarded: boolean;
	draft: boolean;
	deleted: boolean;
	junk: boolean;
	keywords?: string[];
}

export type Priority = "high" | "normal" | "low";

export interface MessageSummary {
	uid: number;
	folder: string;
	messageId: string | null;
	subject: string;
	from: Address[];
	to: Address[];
	cc: Address[];
	date: number;
	size: number;
	flags: Flags;
	hasAttachments: boolean;
	priority: Priority;
	inReplyTo: string | null;
	references?: string[];
	listUnsubscribe: boolean;
}

export type SortField = "date" | "arrival" | "from" | "to" | "subject" | "size";

export interface Sort {
	field: SortField;
	descending: boolean;
}

export type SearchScope = "headers" | "from" | "to" | "subject" | "body" | "everything";

export interface Filter {
	text: string;
	scope: SearchScope;
	unread: boolean;
	flagged: boolean;
	answered: boolean;
	since: string | null;
	before: string | null;
	larger: number | null;
}

export interface MessagePage {
	folder: string;
	uidValidity: number;
	total: number;
	offset: number;
	items: MessageSummary[];
	uids: number[];
}

export interface AttachmentInfo {
	part: string;
	filename: string;
	contentType: string;
	size: number;
	inline: boolean;
	contentId?: string;
}

export interface Message extends MessageSummary {
	replyTo: Address[];
	bcc: Address[];
	html: string | null;
	text: string | null;
	remoteContent: boolean;
	attachments: AttachmentInfo[];
	receiptTo: Address | null;
	unsubscribe: string | null;
	unsubscribePost: boolean;
	headers: [string, string][];
}

export interface MessageRef {
	folder: string;
	uid: number;
}

export type AttachmentSource =
	| { source: "upload"; id: string }
	| { source: "part"; folder: string; uid: number; part: string }
	| { source: "message"; folder: string; uid: number };

export type DraftAttachment = AttachmentSource & {
	filename: string;
	contentType: string;
	size: number;
	inline?: boolean;
	contentId?: string | null;
};

export type ComposeMode = "new" | "reply" | "replyAll" | "forward" | "forwardAttachment" | "editAsNew" | "draft";

export interface Draft {
	mode: ComposeMode;
	identityId: string | null;
	from: Address | null;
	to: Address[];
	cc: Address[];
	bcc: Address[];
	replyTo: Address[];
	subject: string;
	html: string | null;
	text: string;
	attachments: DraftAttachment[];
	source: MessageRef | null;
	inReplyTo: string | null;
	references: string[];
	priority: Priority;
	requestReceipt: boolean;
	draftRef: MessageRef | null;
	messageId: string | null;
}

export interface Identity {
	id: string;
	name: string;
	email: string;
	replyTo: string | null;
	bcc: string | null;
	signature: string;
	isDefault: boolean;
}

export interface Contact {
	id: string;
	name: string;
	emails: string[];
	phones: string[];
	organization: string;
	jobTitle: string;
	notes: string;
	birthday: string | null;
	groups: string[];
	collected: boolean;
	updatedAt: number;
}

export interface Quota {
	used: number;
	limit: number;
}

export interface Upload {
	id: string;
	filename: string;
	contentType: string;
	size: number;
}

export interface FolderCount {
	path: string;
	total: number;
	unread: number;
}

export type MailEvent =
	| { type: "newMail"; folder: string; unread: number; latest: MessageSummary[] }
	| { type: "folderChanged"; folder: string }
	| { type: "counts"; folders: FolderCount[] }
	| { type: "connection"; online: boolean };

export interface SendResult {
	messageId: string;
	saved: boolean;
	warnings?: string[];
}

// ---- filters (sieve.rs) ----

export type FilterField = "from" | "to" | "cc" | "recipient" | "subject" | "body" | "header" | "size";
export type FilterOp = "contains" | "notContains" | "is" | "isNot" | "startsWith" | "endsWith" | "over" | "under";

export interface FilterCondition {
	field: FilterField;
	header: string;
	op: FilterOp;
	value: string;
}

export type FilterAction =
	| { type: "move"; folder: string }
	| { type: "copy"; folder: string }
	| { type: "markRead" }
	| { type: "flag" }
	| { type: "forward"; to: string; keep: boolean }
	| { type: "discard" }
	| { type: "reject"; message: string };

export interface FilterRule {
	id: string;
	name: string;
	enabled: boolean;
	matchAll: boolean;
	conditions: FilterCondition[];
	actions: FilterAction[];
	stop: boolean;
}

export interface Vacation {
	enabled: boolean;
	subject: string;
	body: string;
	days: number;
	start: string | null;
	end: string | null;
	addresses: string[];
}

export interface Filters {
	rules: FilterRule[];
	vacation: Vacation;
}

export interface FilterState {
	filters: Filters;
	foreignScript: string | null;
	foreignContent: string | null;
	extensions: string[];
}

/** Preferences, stored per account on the server or in the desktop database. */
export interface Settings {
	language: "auto" | "pt-BR" | "en";
	theme: "system" | "light" | "dark";
	layout: "columns" | "rows" | "list";
	density: "compact" | "comfortable";
	markReadDelay: number;
	showImages: "never" | "contacts" | "always";
	trustedSenders: string[];
	composeHtml: boolean;
	replyOnTop: boolean;
	saveSent: boolean;
	autosaveSeconds: number;
	confirmPermanentDelete: boolean;
	readReceipts: "ask" | "always" | "never";
	notifications: boolean;
	pageSize: number;
	sort: Sort;
}
