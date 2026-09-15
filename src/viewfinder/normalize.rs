//! Protocol-only response normalization. Missing collections are errors, not empty pages.
use super::Error;
use crate::domain::*;
use serde_json::Value;

pub fn string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
#[track_caller]
pub fn required(v: &Value) -> Result<String, Error> {
    let s = string(v);
    if s.is_empty() {
        tracing::warn!(location = %std::panic::Location::caller(), "Missing required Viewfinder identifier or text");
        Err(Error::Protocol)
    } else {
        Ok(s)
    }
}
#[track_caller]
pub fn array(v: &Value) -> Result<&Vec<Value>, Error> {
    v.as_array().ok_or_else(|| {
        tracing::warn!(location = %std::panic::Location::caller(), "Missing Viewfinder collection");
        Error::Protocol
    })
}
pub fn optional(v: &Value) -> Option<String> {
    let s = string(v);
    (!s.is_empty()).then_some(s)
}
pub fn user(v: &Value) -> Result<User, Error> {
    let id = required(v.get("pk").or_else(|| v.get("id")).unwrap_or(&Value::Null))?;
    Ok(User {
        id,
        username: required(&v["username"])?,
        name: string(&v["full_name"]),
        avatar: optional(&v["profile_pic_url"]),
        biography: string(&v["biography"]),
        followers: v["follower_count"].as_u64().unwrap_or(0),
        following_count: v["following_count"].as_u64().unwrap_or(0),
        posts: v["media_count"].as_u64().unwrap_or(0),
        private: v["is_private"].as_bool().unwrap_or(false),
        relationship: relationship(&v["friendship_status"]),
    })
}
pub fn relationship(v: &Value) -> Relationship {
    if v["outgoing_request"] == true {
        Relationship::Requested
    } else {
        match v["following"].as_bool() {
            Some(true) => Relationship::Following,
            Some(false) => Relationship::NotFollowing,
            None => Relationship::Unknown,
        }
    }
}
pub fn media(v: &Value) -> Media {
    let mut images: Vec<&Value> = v["image_versions2"]["candidates"]
        .as_array()
        .map(|a| a.iter().filter(|i| optional(&i["url"]).is_some()).collect())
        .unwrap_or_default();
    images.sort_by_key(|i| i["width"].as_i64().unwrap_or(0));
    let preview_candidate = images
        .iter()
        .find(|i| i["width"].as_i64().unwrap_or(0) >= 320)
        .or_else(|| images.last());
    let thumbnail = preview_candidate
        .and_then(|i| optional(&i["url"]))
        .or_else(|| optional(&v["thumbnail_src"]))
        .or_else(|| optional(&v["display_url"]));
    let image = images
        .iter()
        .find(|i| i["width"].as_i64().unwrap_or(0) >= 1080)
        .or_else(|| images.last())
        .and_then(|i| optional(&i["url"]))
        .or_else(|| optional(&v["display_url"]));
    // Frames reserve space by aspect ratio; use the rendered candidate's own
    // dimensions so the surface never letterboxes around a mismatched image.
    let candidate_dims = preview_candidate
        .and_then(|i| i["width"].as_i64().zip(i["height"].as_i64()))
        .filter(|(w, h)| *w > 0 && *h > 0);
    let (width, height) = candidate_dims.unwrap_or_else(|| {
        (
            v["original_width"]
                .as_i64()
                .or_else(|| v["dimensions"]["width"].as_i64())
                .unwrap_or(1),
            v["original_height"]
                .as_i64()
                .or_else(|| v["dimensions"]["height"].as_i64())
                .unwrap_or(1),
        )
    });
    Media {
        thumbnail,
        image,
        video: video_url(v),
        width: width.clamp(1, 10000) as i32,
        height: height.clamp(1, 10000) as i32,
    }
}

fn video_url(v: &Value) -> Option<String> {
    let versions = v["video_versions"].as_array();
    // Prefer the smallest rendition at least 720 pixels on its short edge.
    // This handles portrait Reels and landscape videos without trusting API order.
    let mut sized: Vec<_> = versions
        .into_iter()
        .flatten()
        .filter_map(|video| {
            let url = optional(&video["url"])?;
            let edge = video["width"].as_u64()?.min(video["height"].as_u64()?);
            (edge > 0).then_some((edge, url))
        })
        .collect();
    sized.sort_by_key(|(edge, _)| *edge);
    sized
        .iter()
        .find(|(edge, _)| *edge >= 720)
        .or_else(|| sized.last())
        .map(|(_, url)| url.clone())
        .or_else(|| {
            versions
                .into_iter()
                .flatten()
                .find_map(|video| optional(&video["url"]))
        })
        .or_else(|| optional(&v["video_url"]))
}
/// Direct share wrappers nest the media object inconsistently; descend until a
/// node carrying media fields or an identifier is found.
fn share_target(v: &Value) -> &Value {
    let mut node = v;
    loop {
        let looks_like_media = node.get("pk").is_some()
            || node.get("id").is_some()
            || node.get("media_id").is_some()
            || node.get("code").is_some()
            || node["image_versions2"].is_object();
        if looks_like_media {
            return node;
        }
        let inner = ["media", "item", "clip", "video"].iter().find_map(|key| {
            let child = &node[key];
            child.is_object().then_some(child)
        });
        match inner {
            Some(child) => node = child,
            None => return node,
        }
    }
}

pub fn post(v: &Value) -> Result<Post, Error> {
    let v = v
        .get("media_or_ad")
        .or_else(|| v.get("media"))
        .or_else(|| v.get("item"))
        .unwrap_or(v);
    let media = if let Some(items) = v["carousel_media"].as_array() {
        items.iter().map(media).collect()
    } else if let Some(edges) = v["edge_sidecar_to_children"]["edges"].as_array() {
        edges.iter().map(|e| media(&e["node"])).collect()
    } else {
        vec![media(v)]
    };
    Ok(Post {
        id: required(
            v.get("pk")
                .or_else(|| v.get("id"))
                .or_else(|| v.get("media_id"))
                .or_else(|| v.get("code"))
                .or_else(|| v.get("shortcode"))
                .unwrap_or(&Value::Null),
        )?,
        code: string(
            v.get("code")
                .or_else(|| v.get("shortcode"))
                .unwrap_or(&Value::Null),
        ),
        author: user(
            v.get("user")
                .or_else(|| v.get("owner"))
                .unwrap_or(&Value::Null),
        )?,
        caption: optional(&v["caption"]["text"])
            .unwrap_or_else(|| string(&v["edge_media_to_caption"]["edges"][0]["node"]["text"])),
        media,
        liked: v["has_liked"].as_bool().unwrap_or(false),
        likes: v["like_count"]
            .as_u64()
            .or_else(|| v["edge_liked_by"]["count"].as_u64())
            .unwrap_or(0),
        comments: v["comment_count"]
            .as_u64()
            .or_else(|| v["edge_media_to_comment"]["count"].as_u64())
            .unwrap_or(0),
        timestamp: v["taken_at"].as_i64().unwrap_or(0),
    })
}
pub fn posts_connection(v: &Value, paginated: bool) -> Result<Page, Error> {
    let mut items = Vec::new();
    for edge in array(&v["edges"])? {
        let node = &edge["node"];
        // Timeline also contains recommendation modules with no media.
        if node.get("media_or_ad").is_some_and(Value::is_null) {
            continue;
        }
        if node.get("media_or_ad").is_none() && node.get("media").is_some_and(Value::is_null) {
            continue;
        }
        if node.get("media_or_ad").is_none()
            && node.get("media").is_none()
            && node.get("pk").is_none()
            && node.get("id").is_none()
        {
            continue;
        }
        items.push(Item::Post(post(node)?));
    }
    let more = v["page_info"]["has_next_page"].as_bool().unwrap_or(false);
    let next = if more && paginated {
        Some(required(&v["page_info"]["end_cursor"])?)
    } else {
        None
    };
    Ok(Page {
        items,
        next,
        continuation_unavailable: more && !paginated,
    })
}
pub fn comment(v: &Value) -> Result<Comment, Error> {
    Ok(Comment {
        id: required(v.get("pk").or_else(|| v.get("id")).unwrap_or(&Value::Null))?,
        author: user(&v["user"])?,
        text: string(&v["text"]),
        liked: v["has_liked_comment"]
            .as_bool()
            .or_else(|| v["has_liked"].as_bool())
            .unwrap_or(false),
        likes: v["comment_like_count"]
            .as_u64()
            .or_else(|| v["like_count"].as_u64())
            .unwrap_or(0),
    })
}
pub fn conversation(v: &Value) -> Result<Conversation, Error> {
    Ok(Conversation {
        id: required(&v["thread_id"])?,
        thread_fbid: v["thread_fbid"].as_str().map(str::to_owned),
        unread: v["marked_as_unread"].as_bool().unwrap_or(false) || unseen(v),
        title: string(&v["thread_title"]),
        participants: array(&v["users"])?
            .iter()
            .map(user)
            .collect::<Result<_, _>>()?,
        preview: message_preview(&v["items"][0]),
        preview_sender: string(&v["items"][0]["user_id"]),
        preview_timestamp: v["items"][0]["timestamp"].as_i64().unwrap_or(0),
    })
}
/// REST threads carry per-participant `last_seen_at` item markers: the viewer
/// is behind when their marker is absent or predates the newest item.
fn unseen(v: &Value) -> bool {
    let viewer = string(&v["viewer_id"]);
    if viewer.is_empty() || !v["last_seen_at"].is_object() {
        return false;
    }
    let item = if v["last_permanent_item"]["item_id"].is_string() {
        &v["last_permanent_item"]
    } else {
        &v["items"][0]
    };
    let Some(item_id) = item["item_id"].as_str() else {
        return false;
    };
    if string(&item["user_id"]) == viewer {
        return false;
    }
    v["last_seen_at"][viewer.as_str()]["item_id"].as_str() != Some(item_id)
}
fn message_preview(v: &Value) -> String {
    let text = string(&v["text"]);
    if !text.is_empty() {
        return text;
    }
    match v["item_type"].as_str().unwrap_or("") {
        "media_share" | "clip" | "felix_share" => {
            let post = match v["item_type"].as_str() {
                Some("clip") => &v["clip"]["clip"],
                Some("felix_share") => &v["felix_share"]["video"],
                _ => &v["media_share"],
            };
            let author = string(&post["user"]["username"]);
            if author.is_empty() {
                "Shared post".into()
            } else {
                format!("Post by {author}")
            }
        }
        "media" | "visual_media" | "raven_media" => {
            let media = if v["item_type"] == "media" {
                &v["media"]
            } else {
                &v["visual_media"]["media"]
            };
            if media["media_type"] == 2 {
                "Video".into()
            } else {
                let count = media
                    .as_array()
                    .map(Vec::len)
                    .or_else(|| media["carousel_media"].as_array().map(Vec::len))
                    .unwrap_or(1)
                    .max(1);
                format!("{count} {}", if count == 1 { "Image" } else { "Images" })
            }
        }
        "animated_media" => "GIF".into(),
        "sticker" => "Sticker".into(),
        "voice_media" => "Voice message".into(),
        "story_share" | "reel_share" => "Shared story".into(),
        "link" => {
            let title = string(&v["link"]["link_context"]["link_title"]);
            if title.is_empty() {
                "Link".into()
            } else {
                title
            }
        }
        "" => String::new(),
        _ => "Attachment".into(),
    }
}
fn collect_message_media(value: &Value, attachments: &mut Vec<Attachment>) {
    if let Some(items) = value
        .as_array()
        .or_else(|| value["carousel_media"].as_array())
    {
        for item in items {
            collect_message_media(item, attachments);
        }
    } else {
        let media = media(value);
        if media.image.is_some() || media.thumbnail.is_some() || media.video.is_some() {
            attachments.push(Attachment::Media(media));
        }
    }
}

/// Sticker and animated-media items carry renditions under `images` (or flat
/// fields); pick the best displayable URL and the mp4 when present.
fn animated_media(value: &Value) -> Option<Media> {
    let images = &value["images"];
    let rendition = images
        .get("fixed_height")
        .or_else(|| images.get("original"))
        .unwrap_or(value);
    let image = optional(&rendition["webp"])
        .or_else(|| optional(&rendition["url"]))
        .or_else(|| optional(&value["url"]));
    let video = optional(&rendition["mp4"]);
    if image.is_none() && video.is_none() {
        return None;
    }
    Some(Media {
        thumbnail: image.clone(),
        image,
        video,
        width: string(&rendition["width"]).parse().unwrap_or(200),
        height: string(&rendition["height"]).parse().unwrap_or(200),
    })
}

/// Tray stickers report `animated_info` whose rendition layout varies between
/// packs. Scan shallowly for the first node carrying a displayable URL.
pub fn sticker_media(v: &Value) -> Option<Media> {
    fn number(v: &Value) -> Option<i32> {
        v.as_i64()
            .or_else(|| string(v).parse().ok())
            .map(|n| n.clamp(1, 10000) as i32)
    }
    fn scan(v: &Value, depth: u8) -> Option<Media> {
        if depth > 4 {
            return None;
        }
        if let Some(object) = v.as_object() {
            let video = optional(&object["mp4"])
                .or_else(|| optional(&object["animation"]))
                .or_else(|| {
                    object["animation"]
                        .as_object()
                        .and_then(|a| optional(&a["uri"]).or_else(|| optional(&a["url"])))
                });
            let image = optional(&object["webp"])
                .or_else(|| optional(&object["url"]))
                .or_else(|| optional(&object["uri"]))
                .or_else(|| optional(&object["preview"]))
                .or_else(|| optional(&object["preview_url"]));
            if video.is_some() || image.is_some() {
                return Some(Media {
                    thumbnail: image.clone(),
                    image,
                    video,
                    width: number(&object["width"]).unwrap_or(200),
                    height: number(&object["height"]).unwrap_or(200),
                });
            }
            for child in object.values() {
                if let Some(media) = scan(child, depth + 1) {
                    return Some(media);
                }
            }
        } else if let Some(items) = v.as_array() {
            for item in items {
                if let Some(media) = scan(item, depth + 1) {
                    return Some(media);
                }
            }
        }
        None
    }
    scan(v, 0)
}

/// A reply's quoted item is a partial thread item; summarize it the same way
/// inbox previews do, with the text body preferred when present.
fn quoted(v: &Value) -> Option<QuotedMessage> {
    let q = v.get("replied_to_message")?;
    if q.is_null() {
        return None;
    }
    let summary = optional(&q["text"]).unwrap_or_else(|| {
        let preview = message_preview(q);
        if preview.is_empty() {
            "Message".into()
        } else {
            preview
        }
    });
    let mut thumbnails = Vec::new();
    let qkind = q["item_type"].as_str().unwrap_or("");
    match qkind {
        "media" => collect_message_media(&q["media"], &mut thumbnails),
        "visual_media" | "raven_media" | "story_share" | "reel_share" => {
            collect_message_media(&q[qkind]["media"], &mut thumbnails)
        }
        "media_share" | "clip" | "felix_share" => {
            let value = match qkind {
                "clip" => &q["clip"]["clip"],
                "felix_share" => &q["felix_share"]["video"],
                _ => &q["media_share"],
            };
            collect_message_media(value, &mut thumbnails);
        }
        "animated_media" | "sticker" => {
            if let Some(media) = animated_media(&q[qkind]) {
                thumbnails.push(Attachment::Media(media));
            }
        }
        _ => (),
    }
    let thumbnail = thumbnails.into_iter().find_map(|a| match a {
        Attachment::Media(media) => Some(media),
        _ => None,
    });
    Some(QuotedMessage {
        id: string(&q["item_id"]),
        sender: string(&q["user_id"]),
        summary,
        thumbnail,
    })
}

/// Items carry `reactions.emojis` and `reactions.likes` lists keyed by sender.
/// Likes render as a heart reaction.
fn reactions(v: &Value) -> Vec<Reaction> {
    let timestamp = |r: &Value| {
        r["timestamp"]
            .as_i64()
            .or_else(|| string(&r["timestamp"]).parse().ok())
            .unwrap_or(0)
    };
    let mut out = Vec::new();
    if let Some(emojis) = v["reactions"]["emojis"].as_array() {
        for entry in emojis {
            let emoji = string(&entry["emoji"]);
            let sender = string(&entry["sender_id"]);
            if !emoji.is_empty() && !sender.is_empty() {
                out.push(Reaction {
                    emoji,
                    sender,
                    timestamp: timestamp(entry),
                });
            }
        }
    }
    if let Some(likes) = v["reactions"]["likes"].as_array() {
        for entry in likes {
            let sender = string(&entry["sender_id"]);
            if !sender.is_empty() {
                out.push(Reaction {
                    emoji: "♥".into(),
                    sender,
                    timestamp: timestamp(entry),
                });
            }
        }
    }
    out.sort_by_key(|r| r.timestamp);
    out
}

pub fn message(v: &Value) -> Result<Message, Error> {
    let mut attachments = Vec::new();
    let kind = v["item_type"].as_str().unwrap_or("");
    match kind {
        "text" => (),
        "like" => (),
        "media" => collect_message_media(&v["media"], &mut attachments),
        "visual_media" | "raven_media" => {
            let wrapper = v
                .get(kind)
                .or_else(|| v.get("visual_media"))
                .unwrap_or(&Value::Null);
            collect_message_media(&wrapper["media"], &mut attachments);
        }
        "media_share" | "clip" | "felix_share" => {
            let value = match kind {
                "clip" => &v["clip"]["clip"],
                "felix_share" => &v["felix_share"]["video"],
                _ => &v["media_share"],
            };
            let value = share_target(value);
            if let Ok(post) = post(value) {
                attachments.push(Attachment::Post(Box::new(post)));
            } else {
                collect_message_media(value, &mut attachments);
            }
        }
        "story_share" | "reel_share" => {
            let value = share_target(&v[kind]["media"]);
            // Shared stories are full media objects; render as a post card when
            // author and id survive normalization.
            if let Ok(post) = post(value) {
                attachments.push(Attachment::Post(Box::new(post)));
            } else {
                collect_message_media(value, &mut attachments);
            }
        }
        "animated_media" | "sticker" => {
            let value = &v[kind];
            if let Some(media) = animated_media(value) {
                attachments.push(Attachment::Animated {
                    media,
                    alt: optional(&value["alt_text"])
                        .or_else(|| optional(&value["sticker_alt_text"]))
                        .unwrap_or_default(),
                });
            } else {
                collect_message_media(value, &mut attachments);
            }
        }
        "link" => attachments.push(Attachment::Link {
            title: string(&v["link"]["link_context"]["link_title"]),
            url: string(&v["link"]["link_context"]["link_url"]),
            image_url: optional(&v["link"]["link_context"]["link_image_url"]),
        }),
        "voice_media" => {
            let audio = &v["voice_media"]["media"]["audio"];
            let url = audio["audio_urls"]
                .as_array()
                .and_then(|urls| {
                    urls.iter()
                        .find_map(|u| optional(u).or_else(|| optional(&u["url"])))
                })
                .or_else(|| optional(&audio["audio_url"]));
            if let Some(url) = url {
                let media = &v["voice_media"]["media"];
                let duration_ms = media["voice_duration_ms"]
                    .as_i64()
                    .or_else(|| audio["duration"].as_f64().map(|s| (s * 1000.0) as i64))
                    .or_else(|| {
                        let samples = audio["waveform_data"].as_array()?.len() as i64;
                        let hz = audio["waveform_sampling_frequency_hz"].as_i64()?;
                        (hz > 0).then_some(samples * 1000 / hz)
                    })
                    .unwrap_or(0)
                    .max(0) as u64;
                let waveform = audio["waveform_data"]
                    .as_array()
                    .map(|data| {
                        data.iter()
                            .filter_map(|w| w.as_f64().map(|w| w as f32))
                            .collect()
                    })
                    .unwrap_or_default();
                attachments.push(Attachment::Voice {
                    url,
                    duration_ms,
                    waveform,
                });
            } else {
                attachments.push(Attachment::Unavailable);
            }
        }
        _ => attachments.push(Attachment::Unavailable),
    }
    if attachments.is_empty() && !matches!(kind, "text" | "like") {
        attachments.push(Attachment::Unavailable);
    }
    Ok(Message {
        id: required(&v["item_id"])?,
        message_id: v["message_id"].as_str().map(str::to_owned),
        client_context: v["client_context"].as_str().map(str::to_owned),
        seen_by: vec![],
        sender: required(&v["user_id"])?,
        text: if kind == "like" {
            optional(&v["like"]).unwrap_or_else(|| "♥".into())
        } else {
            optional(&v["text"])
                .or_else(|| optional(&v[kind]["text"]))
                .unwrap_or_default()
        },
        timestamp: v["timestamp"].as_i64().unwrap_or(0),
        attachments,
        reply_to: quoted(v),
        reactions: reactions(v),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn dm_media_variants_and_missing_shared_posts_do_not_break_threads() {
        let image = json!({"image_versions2":{"candidates":[{"url":"https://example.com/photo.jpg", "width":400}]}});
        for kind in [
            "media",
            "visual_media",
            "raven_media",
            "story_share",
            "reel_share",
        ] {
            let mut value = json!({"item_id":"1", "user_id":"2", "item_type":kind});
            value[kind] = if kind == "media" {
                image.clone()
            } else {
                json!({"media":image})
            };
            assert!(
                matches!(&message(&value).unwrap().attachments[0], Attachment::Media(m) if m.image.is_some())
            );
        }
        let value = json!({"item_id":"1", "user_id":"2", "item_type":"media", "media":[image.clone(), {"video_versions":[{"url":"https://example.com/video.mp4"}]}]});
        let parsed = message(&value).unwrap();
        assert_eq!(parsed.attachments.len(), 2);
        assert!(matches!(&parsed.attachments[1], Attachment::Media(m) if m.video.is_some()));
        for kind in ["animated_media", "sticker"] {
            let mut value = json!({"item_id":"1", "user_id":"2", "item_type":kind});
            value[kind] = json!({"images":{"fixed_height":{"url":"https://example.com/sticker.gif", "mp4":"https://example.com/sticker.mp4", "width":"200", "height":"100"}}, "alt_text":"cat sticker"});
            assert!(
                matches!(&message(&value).unwrap().attachments[0], Attachment::Animated{media:m, alt} if m.video.is_some() && m.height == 100 && alt == "cat sticker")
            );
        }
        for kind in ["media_share", "clip", "felix_share", "raven_media"] {
            let value = json!({"item_id":"1", "user_id":"2", "item_type":kind});
            assert!(matches!(
                message(&value).unwrap().attachments[0],
                Attachment::Unavailable
            ));
        }
        let post = json!({"pk":"3", "user":{"pk":"4", "username":"alex"}, "carousel_media":[image.clone(), image]});
        let value =
            json!({"item_id":"1", "user_id":"2", "item_type":"media_share", "media_share":post});
        assert!(
            matches!(&message(&value).unwrap().attachments[0], Attachment::Post(p) if p.media.len() == 2)
        );
        // Shared stories render as post cards when the media carries an author.
        let story = json!({"pk":"5", "user":{"pk":"4","username":"alex"}, "image_versions2":{"candidates":[{"url":"https://example.com/s.jpg","width":400}]}});
        let value = json!({"item_id":"1", "user_id":"2", "item_type":"story_share", "story_share":{"media":story}});
        assert!(matches!(
            message(&value).unwrap().attachments[0],
            Attachment::Post(_)
        ));
        let value = json!({"item_id":"1", "user_id":"2", "item_type":"story_share", "story_share":{"media":{"image_versions2":{"candidates":[{"url":"https://example.com/s.jpg","width":400}]}}}});
        assert!(matches!(
            message(&value).unwrap().attachments[0],
            Attachment::Media(_)
        ));
    }

    #[test]
    fn dm_reactions_replies_voice_and_link_images_normalize() {
        let image = json!({"image_versions2":{"candidates":[{"url":"https://example.com/photo.jpg", "width":400}]}});
        let value = json!({
            "item_id":"10", "user_id":"2", "item_type":"text", "text":"hello",
            "reactions": {
                "emojis": [{"sender_id":42, "emoji":"😂", "timestamp":"1788800000000000"}],
                "likes": [{"sender_id":7, "timestamp":1788800001000000_i64}]
            },
            "replied_to_message": {
                "item_id":"9", "user_id":3, "item_type":"media", "media":image
            }
        });
        let parsed = message(&value).unwrap();
        assert_eq!(parsed.reactions.len(), 2);
        assert_eq!(parsed.reactions[0].emoji, "😂");
        assert_eq!(parsed.reactions[0].sender, "42");
        assert_eq!(parsed.reactions[1].emoji, "♥");
        let quote = parsed.reply_to.unwrap();
        assert_eq!(quote.id, "9");
        assert_eq!(quote.sender, "3");
        assert_eq!(quote.summary, "1 Image");
        assert!(quote.thumbnail.is_some());
        let value = json!({
            "item_id":"10", "user_id":"2", "item_type":"text", "text":"same",
            "replied_to_message": {"item_id":"8", "user_id":3, "item_type":"text", "text":"earlier"}
        });
        let quote = message(&value).unwrap().reply_to.unwrap();
        assert_eq!(quote.summary, "earlier");
        assert!(quote.thumbnail.is_none());
        let value = json!({"item_id":"11", "user_id":"2", "item_type":"voice_media",
        "voice_media":{"media":{"voice_duration_ms":4200,"audio":{
            "audio_urls":[{"url":"https://example.com/voice.m4a"}],
            "waveform_data":[0.1, 0.9, 0.4],
            "waveform_sampling_frequency_hz":50
        }}}});
        assert!(
            matches!(&message(&value).unwrap().attachments[0], Attachment::Voice{url, duration_ms: 4200, waveform} if url == "https://example.com/voice.m4a" && waveform.len() == 3)
        );
        let value = json!({"item_id":"12", "user_id":"2", "item_type":"link",
            "link":{"link_context":{"link_title":"A page","link_url":"https://example.com","link_image_url":"https://example.com/i.jpg"}}});
        assert!(
            matches!(&message(&value).unwrap().attachments[0], Attachment::Link{image_url, ..} if image_url.as_deref() == Some("https://example.com/i.jpg"))
        );
    }

    #[test]
    fn conversation_previews_describe_attachments_and_preserve_sender() {
        for (item, expected) in [
            (json!({"item_type":"text", "text":"Hello"}), "Hello"),
            (
                json!({"item_type":"media_share", "media_share":{"user":{"username":"alex"}}}),
                "Post by alex",
            ),
            (
                json!({"item_type":"clip", "clip":{"clip":{"user":{"username":"sam"}}}}),
                "Post by sam",
            ),
            (json!({"item_type":"media_share"}), "Shared post"),
            (
                json!({"item_type":"media", "media":{"media_type":1}}),
                "1 Image",
            ),
            (
                json!({"item_type":"media", "media":{"carousel_media":[{}, {}]}}),
                "2 Images",
            ),
            (
                json!({"item_type":"media", "media":{"media_type":2}}),
                "Video",
            ),
        ] {
            let mut item = item;
            item["user_id"] = json!(42);
            item["timestamp"] = json!(1788800000000000_i64);
            let thread =
                conversation(&json!({"thread_id":"1", "users":[], "items":[item]})).unwrap();
            assert_eq!(thread.preview, expected);
            assert_eq!(thread.preview_sender, "42");
            assert_eq!(thread.preview_timestamp, 1788800000000000);
        }
        assert_eq!(message_preview(&Value::Null), "");
    }
    #[test]
    fn conversation_unread_uses_last_seen_marker() {
        let thread = |seen: &str| {
            conversation(&json!({
                "thread_id":"1", "viewer_id": 7, "users":[],
                "last_permanent_item": {"item_id":"m2", "user_id": 9},
                "last_seen_at": {"7": {"item_id": seen}},
                "items": [{"item_id":"m2", "user_id": 9, "item_type":"text"}]
            }))
            .unwrap()
            .unread
        };
        assert!(thread("m1"));
        assert!(!thread("m2"));
        assert!(
            conversation(&json!({
                "thread_id":"1", "viewer_id": 7, "users":[],
                "last_permanent_item": {"item_id":"m2", "user_id": 9},
                "last_seen_at": {"9": {"item_id": "m2"}},
                "items": []
            }))
            .unwrap()
            .unread
        );
        assert!(
            !conversation(&json!({
                "thread_id":"1", "viewer_id": 7, "users":[],
                "last_permanent_item": {"item_id":"m2", "user_id": 7},
                "last_seen_at": {},
                "items": []
            }))
            .unwrap()
            .unread
        );
        assert!(
            !conversation(&json!({"thread_id":"1", "users":[], "items": []}))
                .unwrap()
                .unread
        );
    }
    #[test]
    fn video_renditions_balance_resolution_and_transfer_size() {
        for portrait in [true, false] {
            let versions: Vec<_> = [(1080, 1920), (480, 854), (720, 1280)]
                .into_iter()
                .map(|(short, long)| {
                    json!({
                        "width": if portrait { short } else { long },
                        "height": if portrait { long } else { short },
                        "url": format!("video-{short}")
                    })
                })
                .collect();
            assert_eq!(
                media(&json!({"video_versions": versions})).video.as_deref(),
                Some("video-720")
            );
        }
        assert_eq!(
            media(&json!({"video_versions": [
                {"width": 720, "height": 1280},
                {"width": 320, "height": 568, "url": "small"},
                {"width": 480, "height": 854, "url": "medium"}
            ]}))
            .video
            .as_deref(),
            Some("medium")
        );
        assert_eq!(
            media(&json!({"video_versions": [{}, {"url": "unknown-size"}]}))
                .video
                .as_deref(),
            Some("unknown-size")
        );
        assert_eq!(
            media(&json!({"video_url": "fallback"})).video.as_deref(),
            Some("fallback")
        );
    }
    #[test]
    fn documented_media_variants() {
        let value: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/posts.json")).unwrap();
        let page = posts_connection(&value, true).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.next.as_deref(), Some("opaque-cursor"));
        let Item::Post(p) = &page.items[0] else {
            panic!()
        };
        assert_eq!(p.media.len(), 2);
        assert_eq!(p.author.id, "101");
        assert_eq!(p.likes, 0);
        assert!(p.media[1].video.is_some());
        let Item::Post(p) = &page.items[1] else {
            panic!()
        };
        assert_eq!(p.caption, "Fixture caption");
    }
    #[test]
    fn timeline_skips_null_media_modules_and_keeps_pagination() {
        let page = posts_connection(
            &json!({
                "edges": [
                    {"node": {"id": "module", "media": null}},
                    {"node": {"media_or_ad": null}},
                    {"node": {"media": {"pk": "42", "user": {"pk": "7", "username": "fixture"}}}}
                ],
                "page_info": {"has_next_page": true, "end_cursor": "next"}
            }),
            true,
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next.as_deref(), Some("next"));
        assert!(posts_connection(&json!({"edges": [{"node": {"media": {}}}]}), true).is_err());
    }
    #[test]
    fn null_and_missing_are_not_empty_success() {
        assert!(posts_connection(&json!({}), true).is_err());
        assert!(post(&json!({"pk": "1"})).is_err());
        assert!(
            posts_connection(
                &json!({"edges": [], "page_info":{"has_next_page":true}}),
                true
            )
            .is_err()
        );
    }
    #[test]
    fn unknown_message_is_explicit() {
        let m = message(&json!({"item_id":"1","user_id":2,"item_type":"future_type"})).unwrap();
        assert!(matches!(m.attachments[0], Attachment::Unavailable));
    }
}
