/**
 * Cues for Pi Workbench's pet, Glitch.
 *
 * - `/kill` gets the pet shot and thrown across the screen.
 * - `/pet-say <text>` has it say something in its bubble.
 *
 * The workbench listens for the `pet` status line, which this sets for a
 * moment. Other Pi clients just show it briefly in their footer. Without
 * this extension the workbench still takes `/kill` itself.
 *
 * Install: copy into ~/.pi/agent/extensions/ (or a project's
 * .pi/extensions/), then reload Pi.
 */
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

/** Sets the `pet` status; the time makes every cue distinct. */
function cue(ctx: ExtensionContext, text: string) {
	// Held now: by the time the status clears, the session may have changed
	// and the context gone stale.
	const ui = ctx.ui;
	ui.setStatus("pet", text);
	setTimeout(() => {
		try {
			ui.setStatus("pet", undefined);
		} catch {
			// The session it belonged to has gone; nothing to clear.
		}
	}, 1500);
}

export default function (pi: ExtensionAPI) {
	pi.registerCommand("kill", {
		description: "Shoot the workbench's pet",
		handler: async (_args, ctx) => cue(ctx, `kill:${Date.now()}`),
	});
	pi.registerCommand("pet-say", {
		description: "Have the workbench's pet say something",
		handler: async (args, ctx) => {
			const text = (args ?? "").trim();
			if (text) cue(ctx, `say:${text}`);
		},
	});
}
