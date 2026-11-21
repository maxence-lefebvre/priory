// The review guide in a PR description and its quiz: page content, so every check bounds what is drawn.

const MAX_BYTES = 8 * 1024;
const MAX_STEPS = 8;
const MAX_QUESTIONS = 4;

const text = (value) => typeof value === "string" && value.length > 0 && value.length <= 500;

/** The guide in `json`, the block's text, or why it does not parse. @param {string} json */
export function parseGuide(json) {
  if (new TextEncoder().encode(json).length > MAX_BYTES) return { error: "the block is over 8 KiB" };
  let guide;
  try {
    guide = JSON.parse(json);
  } catch (error) {
    return { error: `the JSON does not parse (${error.message})` };
  }
  const why = problem(guide);
  return why ? { error: why } : { guide };
}

function problem(guide) {
  if (guide?.version !== 1) return "version is not 1";
  if (typeof guide.head !== "string" || !/^[0-9a-f]{7,40}$/.test(guide.head)) return "head is not a commit sha";
  if (!Array.isArray(guide.map) || guide.map.length > 4 || !guide.map.every(text)) return "map is not up to 4 lines of text";
  if (!Array.isArray(guide.steps) || guide.steps.length > MAX_STEPS) return "steps is not a list of at most 8 steps";
  for (const step of guide.steps) {
    if (!text(step?.title) || !text(step.file)) return "a step lacks its title or file";
    if (step.lines !== undefined && !/^\d+(-\d+)?$/.test(step.lines)) return `the lines of step "${step.title}" are not N or N-M`;
  }
  if (guide.quiz === undefined) return null;
  if (typeof guide.quiz !== "object" || guide.quiz === null) return "the quiz is not an object";
  const { pass, questions } = guide.quiz;
  if (!Array.isArray(questions) || questions.length === 0 || questions.length > MAX_QUESTIONS) return "the quiz is not 1 to 4 questions";
  if (!Number.isInteger(pass) || pass < 1 || pass > questions.length) return "the quiz's pass is not between 1 and its question count";
  for (const question of questions) {
    if (!text(question?.id) || !text(question.question)) return "a question lacks its id or text";
    if (!Array.isArray(question.options) || question.options.length < 2 || question.options.length > 6 || !question.options.every(text)) {
      return `question ${question.id} does not have 2 to 6 options`;
    }
    if (typeof question.answer !== "string" || !/^sha256:[0-9a-f]{16}$/.test(question.answer)) return `question ${question.id}'s answer is not sha256: and 16 hex digits`;
    if (question.cite !== undefined && !text(question.cite)) return `question ${question.id}'s cite is not text`;
  }
  return null;
}

async function sha256(value) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** The hash a question's `answer` holds for `option`. */
export const answerHash = async (pr, id, option) => `sha256:${(await sha256(`${pr.repo}#${pr.number}:${id}:${option}`)).slice(0, 16)}`;

/** How `answers`, an option per question id, score against the quiz. */
export async function grade(guide, pr, answers) {
  const correct = {};
  for (const question of guide.quiz.questions) {
    correct[question.id] = answers[question.id] !== undefined && (await answerHash(pr, question.id, answers[question.id])) === question.answer;
  }
  const right = Object.values(correct).filter(Boolean).length;
  return { right, total: guide.quiz.questions.length, passed: right >= guide.quiz.pass, correct };
}

/** Where a score is kept: one pull request, one head sha, one guide text. */
export const scoreKey = async (pr, head, json) => `score:${pr.repo}#${pr.number}@${head}:${(await sha256(json)).slice(0, 16)}`;
