import { div, image } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";

export function versionHeader(t, note) {
  const [year, month, day] = note.date.split("-");
  const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const date = year && months[Number(month) - 1] && Number(day)
    ? `${months[Number(month) - 1]} ${Number(day)}` : note.date;
  return h_flex().gap(10).items_center()
    .child(div().text_size(13).font_medium().text_color(t.muted).child(note.version))
    .child(div().text_size(12).text_color(t.dim).child(date));
}

export function releaseItems(t, note, { collapsed = false, spotlight = false } = {}) {
  const body = v_flex().gap(collapsed ? 4 : 20);
  const highlights = note.items.filter(item => !item.fixed);
  const hero = spotlight ? highlights[0]?.media[0] : null;
  if (hero && !collapsed) body.child(image(hero).w_full().h(260));
  for (const item of highlights) {
    const row = v_flex().gap(5);
    if (item.headline) row.child(div().text_size(collapsed ? 12 : 14).font_semibold().text_color(t.foreground).child(item.headline));
    if (item.detail && (!collapsed || !item.headline)) row.child(div().text_size(collapsed ? 12 : 13).line_height("20px").text_color(t.muted).child(item.detail));
    if (!collapsed) for (const url of item.media) {
      if (url !== hero) row.child(image(url).w_full().h(220));
    }
    body.child(row);
  }
  const fixes = note.items.filter(item => item.fixed);
  if (fixes.length && !collapsed) {
    const list = v_flex().gap(5)
      .child(div().mb(3).text_size(11).font_medium().text_color(t.dim).child("Also fixed"));
    for (const item of fixes) {
      list.child(div().text_size(12).line_height("17px").text_color(t.muted).child(`• ${[item.headline, item.detail].filter(Boolean).join(" ")}`));
      for (const url of item.media) list.child(image(url).w_full().h(180));
    }
    body.child(list);
  }
  return body;
}
