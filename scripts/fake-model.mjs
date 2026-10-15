#!/usr/bin/env node
// Scripted OpenAI-compatible model for exercising Pi without a real provider.
//
// A prompt that is a JSON array of steps drives the run. Each model request
// answers with the next step after the latest user message:
//   [{"tool": "edit", "args": {"path": "a.rs", "edits": [{"oldText": "x", "newText": "y"}]}},
//    {"text": "Done."}]
// A step may also be {"tools": [{tool, args}, ...]} for parallel calls.
// Any other prompt gets an echo reply. Tool arguments stream in small chunks
// with a delay so clients can observe live tool-call deltas.
//
//   node scripts/fake-model.mjs [port]   (default 0 = any free port)
// Prints "listening <port>" once ready.
import { createServer } from "node:http";

const CHUNK = 12;
const DELAY_MS = Number(process.env.FAKE_MODEL_DELAY_MS ?? 25);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function text(content) {
	if (typeof content === "string") return content;
	return (content ?? []).map((part) => part.text ?? "").join("");
}

function nextStep(messages) {
	const lastUser = messages.findLastIndex((m) => m.role === "user");
	const prompt = text(messages[lastUser]?.content).trim();
	let steps;
	try {
		steps = JSON.parse(prompt);
	} catch {
		return { text: `fake model received: ${prompt}` };
	}
	if (!Array.isArray(steps)) return { text: "Expected a JSON array of steps." };
	const answered = messages.slice(lastUser + 1).filter((m) => m.role === "assistant").length;
	return steps[answered] ?? { text: "Done." };
}

function chunk(delta, finish = null) {
	const body = {
		id: "fake",
		object: "chat.completion.chunk",
		created: 0,
		model: "fake",
		choices: [{ index: 0, delta, finish_reason: finish }],
	};
	return `data: ${JSON.stringify(body)}\n\n`;
}

async function stream(res, step) {
	res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
	res.write(chunk({ role: "assistant", content: "" }));
	const calls = step.tools ?? (step.tool ? [{ tool: step.tool, args: step.args }] : []);
	if (calls.length === 0) {
		for (let i = 0; i < (step.text ?? "").length; i += CHUNK) {
			res.write(chunk({ content: step.text.slice(i, i + CHUNK) }));
			await sleep(DELAY_MS);
		}
		res.write(chunk({}, "stop"));
	} else {
		for (const [index, call] of calls.entries()) {
			const id = `call_${Date.now()}_${index}`;
			res.write(chunk({ tool_calls: [{ index, id, type: "function", function: { name: call.tool, arguments: "" } }] }));
			const args = JSON.stringify(call.args ?? {});
			for (let i = 0; i < args.length; i += CHUNK) {
				res.write(chunk({ tool_calls: [{ index, function: { arguments: args.slice(i, i + CHUNK) } }] }));
				await sleep(DELAY_MS);
			}
		}
		res.write(chunk({}, "tool_calls"));
	}
	res.write("data: [DONE]\n\n");
	res.end();
}

const server = createServer((req, res) => {
	let body = "";
	req.on("data", (data) => (body += data));
	req.on("end", () => {
		if (req.method === "GET" && req.url.endsWith("/models")) {
			res.writeHead(200, { "content-type": "application/json" });
			res.end(JSON.stringify({ object: "list", data: [{ id: "fake", object: "model" }] }));
			return;
		}
		if (req.method !== "POST" || !req.url.endsWith("/chat/completions")) {
			res.writeHead(404).end();
			return;
		}
		const request = JSON.parse(body || "{}");
		stream(res, nextStep(request.messages ?? [])).catch(() => res.destroy());
	});
});

server.listen(Number(process.argv[2] ?? 0), "127.0.0.1", () => {
	console.log(`listening ${server.address().port}`);
});
