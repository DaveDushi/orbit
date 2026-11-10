/**
 * The steps of a run as people read them: a step repeated right after itself shows once, with its count
 * ("Using shopify → api request ×3"). The order stays; a step that comes back later starts a new line.
 */
export function groupSteps(steps: readonly string[]): string[] {
	const groups: { step: string; count: number }[] = [];
	for (const step of steps) {
		const last = groups.at(-1);
		if (last?.step === step) last.count += 1;
		else groups.push({ step, count: 1 });
	}
	return groups.map(({ step, count }) => (count > 1 ? `${step} ×${count}` : step));
}
