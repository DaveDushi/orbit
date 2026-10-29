/**
 * Orbit chat bodies carry mentions as tokens: `<@user_id>`, `<#conversation_id>`, `<!channel>`, `<!here>`.
 * The model reads and writes `@Name` instead.
 */

export type Person = { readonly id: string; readonly name: string };

const USER_TOKEN = /<@([0-9a-f-]{36})>/gi;

/** Orbit body → text for the model. Unknown users stay as `@unknown`. */
export function tokensToNames(body: string, people: readonly Person[]): string {
	const names = new Map(people.map((person) => [person.id.toLowerCase(), person.name]));
	return body
		.replace(USER_TOKEN, (_, id: string) => `@${names.get(id.toLowerCase()) ?? "unknown"}`)
		.replace(/<#[0-9a-f-]{36}>/gi, "#channel")
		.replace(/<!channel>/g, "@channel")
		.replace(/<!here>/g, "@here");
}

/**
 * Model text → Orbit body: `@Name` of a known person becomes their token. The longest name wins, so
 * "@Ann Lee" is not read as "@Ann". Names match whole words only and ignore case. `@channel` and
 * `@here` stay plain text: the agent never notifies a whole channel.
 */
export function namesToTokens(text: string, people: readonly Person[]): string {
	const sorted = [...people].filter((person) => person.name.trim()).sort((a, b) => b.name.length - a.name.length);
	let result = "";
	let index = 0;
	outer: while (index < text.length) {
		const at = text.indexOf("@", index);
		if (at < 0) break;
		result += text.slice(index, at);
		// `name@example.com` is an e-mail address, not a mention.
		const before = at > 0 ? text[at - 1] : " ";
		if (/[\p{L}\p{N}_.]/u.test(before)) {
			result += "@";
			index = at + 1;
			continue;
		}
		for (const person of sorted) {
			const candidate = text.slice(at + 1, at + 1 + person.name.length);
			const after = text[at + 1 + person.name.length] ?? " ";
			if (candidate.toLowerCase() === person.name.toLowerCase() && !/[\p{L}\p{N}_]/u.test(after)) {
				result += `<@${person.id}>`;
				index = at + 1 + person.name.length;
				continue outer;
			}
		}
		result += "@";
		index = at + 1;
	}
	return result + text.slice(index);
}

/** Splits a long body into chunks of at most `limit` characters, at line breaks where it can. */
export function splitBody(body: string, limit = 3800): string[] {
	const chunks: string[] = [];
	let rest = body;
	while (rest.length > limit) {
		let cut = rest.lastIndexOf("\n", limit);
		if (cut < limit / 2) cut = limit;
		chunks.push(rest.slice(0, cut).trimEnd());
		rest = rest.slice(cut).replace(/^\n+/, "");
	}
	chunks.push(rest);
	return chunks;
}
