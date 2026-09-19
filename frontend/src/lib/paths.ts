/** Strip the server-side user home prefix from a path, replacing it with ~  */
export function prettifyPath(path: string): string {
  const home = localStorage.getItem('flamel-user-home');
  if (home && path.startsWith(home)) {
    const rel = path.slice(home.length);
    return '~' + (rel.startsWith('/') ? rel : '/' + rel);
  }
  return path;
}
