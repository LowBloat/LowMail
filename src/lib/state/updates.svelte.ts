/**
 * Checks GitHub Releases for a signed update. Installed copies download,
 * verify, install and restart; other formats get a download link.
 */
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { isDesktop, isMobile, openExternal } from "../api";
import { errorText, t } from "../i18n/index.svelte";

export interface UpdatePolicy {
	/** installer, appimage, deb, rpm, package, portable or mobile. */
	channel: string;
	canInstall: boolean;
	target: string | null;
}

const RELEASES_URL = "https://github.com/LowBloat/LowMail/releases/latest";
const FIRST_CHECK_MS = 5_000;
const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;

class Updates {
	currentVersion = $state("");
	policy = $state<UpdatePolicy | null>(null);
	available = $state<{ version: string; notes: string } | null>(null);
	checking = $state(false);
	installing = $state(false);
	/** Download progress from 0 to 1, or null when the size is unknown. */
	progress = $state<number | null>(null);
	error = $state<string | null>(null);
	dialogOpen = $state(false);
	/** Set after a check the user asked for found nothing new. */
	upToDate = $state(false);
	private update: Update | null = null;
	private timer: ReturnType<typeof setInterval> | undefined;

	/** Self-update exists in the desktop app only. */
	get supported() {
		return isDesktop && !isMobile;
	}

	/** Why this copy cannot update itself, shown next to the download link. */
	get manualHint(): string | null {
		if (!this.policy || this.policy.canInstall) return null;
		const known = ["deb", "rpm", "package", "appimage"];
		return t(`updates.manual.${known.includes(this.policy.channel) ? this.policy.channel : "portable"}`);
	}

	/** Starts background checks; safe to call once at startup. */
	start() {
		if (!this.supported || this.timer) return;
		getVersion()
			.then((v) => (this.currentVersion = v))
			.catch(() => {});
		setTimeout(() => this.check(false), FIRST_CHECK_MS);
		this.timer = setInterval(() => this.check(false), CHECK_EVERY_MS);
	}

	/** `interactive`: the user asked, so report "up to date" and errors. */
	async check(interactive: boolean): Promise<void> {
		if (!this.supported || this.checking || this.installing) return;
		this.checking = true;
		this.error = null;
		this.upToDate = false;
		try {
			this.policy ??= await invoke<UpdatePolicy>("update_policy");
			// Packages that cannot install still read the AppImage entry to learn
			// the version and notes.
			const target = this.policy.target ?? (this.policy.canInstall ? undefined : "linux-x86_64-appimage");
			const update = await check(target ? { target } : undefined);
			this.update = update;
			this.available = update ? { version: update.version, notes: update.body ?? "" } : null;
			if (update && interactive) this.dialogOpen = true;
			if (!update && interactive) this.upToDate = true;
		} catch {
			// GitHub unreachable, or no release yet: the updater's own text is technical.
			if (interactive) this.error = t("updates.checkFailed");
		} finally {
			this.checking = false;
		}
	}

	async install(): Promise<void> {
		if (!this.update || !this.policy?.canInstall) return;
		this.installing = true;
		this.error = null;
		this.progress = null;
		let total = 0;
		let received = 0;
		try {
			await this.update.downloadAndInstall((event) => {
				if (event.event === "Started") total = event.data.contentLength ?? 0;
				else if (event.event === "Progress") {
					received += event.data.chunkLength;
					this.progress = total > 0 ? Math.min(1, received / total) : null;
				} else if (event.event === "Finished") this.progress = 1;
			});
			await relaunch();
		} catch (e) {
			this.error = errorText(e);
			this.installing = false;
		}
	}

	download() {
		void openExternal(RELEASES_URL);
	}
}

export const updates = new Updates();
