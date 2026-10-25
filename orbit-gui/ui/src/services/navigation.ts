export function instancePath(id: string, tab = "mods") {
  return `/instance/${encodeURIComponent(id)}/${tab}`;
}
export function discoveryPath(id?: string) {
  return id ? `/browse?instance=${encodeURIComponent(id)}` : "/browse";
}
