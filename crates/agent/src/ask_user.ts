// The workbench's question tool. Pi's own question tools draw in its
// terminal, which a session run by the workbench doesn't have, so this one
// asks wb-agent instead: it writes the questions as one line to the socket
// in WB_ASK, and wb-agent writes back the answers as one line once you've
// answered them in the workbench.

import { connect } from 'node:net';
import type { ExtensionAPI } from '@earendil-works/pi-coding-agent';
import { Type } from 'typebox';

const Choice = Type.Object({
  label: Type.String({ description: 'The answer, short' }),
  description: Type.Optional(Type.String({ description: 'What picking it means, in a sentence' })),
});

const Question = Type.Object({
  label: Type.Optional(Type.String({ description: "A word or two for the question's tab, e.g. 'Scope'" })),
  question: Type.String({ description: 'The question, in full' }),
  choices: Type.Array(Choice, { description: 'Suggested answers, best first' }),
  allowOther: Type.Optional(
    Type.Boolean({ description: 'Let the user write an answer of their own (default: true)' }),
  ),
});

const Params = Type.Object({
  questions: Type.Array(Question, { description: 'One question, or a few that belong together' }),
});

interface Asked {
  label?: string;
  question: string;
  choices: { label: string; description?: string }[];
  allowOther?: boolean;
}

const say = (text: string, details: Record<string, unknown> = {}) => ({
  content: [{ type: 'text' as const, text }],
  details,
});

/** Asks wb-agent, and waits for the answers: one per question, or none if dismissed. */
function ask(path: string, questions: Asked[], signal?: AbortSignal) {
  return new Promise<{ answers?: string[]; error?: string }>((resolve) => {
    const socket = connect(path);
    let buffer = '';
    let done = false;
    const finish = (result: { answers?: string[]; error?: string }) => {
      if (done) return;
      done = true;
      signal?.removeEventListener('abort', stop);
      socket.destroy();
      resolve(result);
    };
    const stop = () => finish({ error: 'The run was stopped before the user answered.' });
    signal?.addEventListener('abort', stop);
    socket.setEncoding('utf8');
    socket.on('connect', () => {
      const asked = questions.map((q, i) => ({
        label: q.label?.trim() || `Q${i + 1}`,
        question: q.question,
        choices: q.choices.map((c) => ({ label: c.label, description: c.description ?? null })),
        other: q.allowOther !== false,
      }));
      socket.write(`${JSON.stringify({ questions: asked })}\n`);
    });
    socket.on('data', (chunk: string) => {
      buffer += chunk;
      const end = buffer.indexOf('\n');
      if (end < 0) return;
      try {
        const reply = JSON.parse(buffer.slice(0, end));
        finish(Array.isArray(reply.answers) ? { answers: reply.answers.map(String) } : {});
      } catch {
        finish({ error: 'The workbench sent an answer this tool could not read.' });
      }
    });
    socket.on('error', (err) => finish({ error: `Could not reach the workbench: ${err.message}` }));
    socket.on('close', () => finish({ error: 'The workbench closed the question without an answer.' }));
  });
}

export default function askUser(pi: ExtensionAPI) {
  pi.registerTool({
    name: 'ask_user',
    label: 'Ask the user',
    description:
      'Ask the user one or more questions and wait for the answers. Use it to clarify requirements, ' +
      'get preferences, or settle a decision before going on. Give each question a few suggested ' +
      'answers; the user can pick one or write their own. Ask several at once only when they belong together.',
    parameters: Params,

    async execute(_toolCallId, params, signal) {
      const questions = params.questions as Asked[];
      const path = process.env.WB_ASK;
      if (!path) return say('Error: this session is not running in the workbench, so there is no one to ask.');
      if (questions.length === 0) return say('Error: no questions given.');

      const reply = await ask(path, questions, signal);
      if (reply.error) return say(reply.error, { questions, cancelled: true });
      if (!reply.answers) return say('The user dismissed the question without answering.', { questions, cancelled: true });
      const answers = reply.answers;
      const text =
        questions.length === 1
          ? `The user answered: ${answers[0]}`
          : questions.map((q, i) => `${q.label?.trim() || `Q${i + 1}`}: ${answers[i] ?? ''}`).join('\n');
      return say(text, { questions, answers, cancelled: false });
    },
  });
}
