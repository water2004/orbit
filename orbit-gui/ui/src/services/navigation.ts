export function instancePath(id: string, tab = "mods") {
  return `/instance/${encodeURIComponent(id)}/${tab}`;
}
