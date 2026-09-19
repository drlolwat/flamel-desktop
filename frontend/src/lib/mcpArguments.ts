/** Parse argv, not a shell command: never expand variables or execute text. */
export function parseMcpArguments(input: string): string[] {
  const text = input.trim();
  if (text.startsWith('[')) {
    let value: unknown;
    try { value = JSON.parse(text); }
    catch { throw new Error('Arguments must be a valid JSON array of strings.'); }
    if (!Array.isArray(value) || !value.every((item) => typeof item === 'string')) {
      throw new Error('Arguments must be a JSON array of strings.');
    }
    return value;
  }
  const args: string[] = [];
  let word = '', quote = '', started = false;
  for (let i = 0; i < text.length; i++) {
    const char = text[i], next = text[i + 1];
    if (char === '\\' && next && ((quote && next === quote) || (!quote && /[\s"']/.test(next)))) {
      word += next; started = true; i++;
    } else if (quote) {
      if (char === quote) quote = '';
      else word += char;
    } else if (char === '"' || char === "'") {
      quote = char; started = true;
    } else if (/\s/.test(char)) {
      if (started) args.push(word);
      word = ''; started = false;
    } else {
      word += char; started = true;
    }
  }
  if (quote) throw new Error('Close the quote in Arguments, or use a JSON array of strings.');
  if (started) args.push(word);
  return args;
}
