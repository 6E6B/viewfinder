//! Identifiers and variables transcribed from the supplied Python reference and addenda.
//! No browser probing or automatically retried mutations.
use super::{Client, Error, normalize as n, validate_id};
use crate::domain::*;
use serde_json::json;
use std::sync::Arc;

fn confirmed_like(
    v: &serde_json::Value,
    key: &str,
    typename: &str,
    liked: bool,
) -> Result<bool, Error> {
    if v["data"][key]["__typename"].as_str() == Some(typename) {
        Ok(liked)
    } else {
        tracing::warn!(key, expected_type = typename, response = %super::diagnostics::shape(v), "Like response did not confirm the action");
        Err(Error::Protocol)
    }
}

impl Client {
    /// Inbox, thread and send paths all need the same mailbox snapshot within
    /// seconds of each other; hold the lock across the fetch so concurrent
    /// callers share one request instead of duplicating it.
    async fn dm_mailbox(&self) -> Result<Arc<serde_json::Value>, Error> {
        const FRESH: std::time::Duration = std::time::Duration::from_secs(30);
        let mut cached = self.mailbox_cache.lock().await;
        if let Some((fetched, mailbox)) = cached.as_ref()
            && fetched.elapsed() < FRESH
        {
            return Ok(mailbox.clone());
        }
        let device = self.device_id.as_ref().ok_or(Error::SessionFile)?;
        let v = self
            .graphql(
                "28794932076791671",
                json!({
                    "device_id_for_iris_subscription": device,
                    "__relay_internal__pv__IGDIsProfessionalAccountGKrelayprovider": false,
                    "__relay_internal__pv__IGDPinnedThreadsRenderEnabledGKrelayprovider": true,
                    "__relay_internal__pv__IGDMaxUnreadMessagesCountrelayprovider": 5,
                    "__relay_internal__pv__IGDThreadListActionsEnabledGKrelayprovider": true
                }),
                true,
            )
            .await?;
        let mailbox = &v["data"]["get_slide_mailbox_for_iris_subscription"];
        n::array(&mailbox["threads_by_folder"]["edges"])?;
        let mailbox = Arc::new(mailbox.clone());
        *cached = Some((std::time::Instant::now(), mailbox.clone()));
        Ok(mailbox)
    }

    /// Thread history from the slide (msys) channel. `direct_v2` masks newer
    /// item types as `placeholder`; slide messages carry the real content.
    async fn thread_slide(&self, fbid: Option<&str>) -> Result<serde_json::Value, Error> {
        let Some(fbid) = fbid else {
            return Err(Error::Protocol);
        };
        let v = self
            .graphql(
                "28047224604940200",
                json!({
                    "min_uq_seq_id": null,
                    "thread_fbid": fbid,
                    "__relay_internal__pv__IGDEnableOffMsysChatThemesQErelayprovider": false,
                    "__relay_internal__pv__IGDInitialMessagePageCountrelayprovider": 20
                }),
                true,
            )
            .await?;
        Ok(v["data"]["get_slide_thread_nullable"]["as_ig_direct_thread"].clone())
    }

    pub async fn load(&self, route: Route, cursor: Option<String>) -> Result<Page, Error> {
        match route {
            Route::Home => {
                let mut variables = json!({
                    "first":12,"variant":"home",
                    "__relay_internal__pv__PolarisMultiCaptionCarouselEnabledrelayprovider":true,
                    "__relay_internal__pv__PolarisShortDramaEnabledrelayprovider":false,
                    "__relay_internal__pv__PolarisReelsRecoDebugOverlayEnabledrelayprovider":false,
                    "__relay_internal__pv__PolarisAdDebugToolEnabledrelayprovider":false
                });
                let device = self.device_id.as_ref().ok_or(Error::SessionFile)?;
                let id = if let Some(cursor) = cursor {
                    variables["after"] = json!(cursor);
                    variables["before"] = json!(null);
                    variables["last"] = json!(null);
                    variables["data"] = json!({"device_id":device,"feed_view_info":"[]"});
                    "28744841435121707"
                } else {
                    variables["data"] = json!({"device_id":device,"is_async_ads_double_request":"0","is_async_ads_in_headload_enabled":"0","is_async_ads_rti":"0","rti_delivery_backend":"0"});
                    "28629425830015528"
                };
                let v = self.graphql(id, variables, false).await?;
                n::posts_connection(&v["data"]["xdt_api__v1__feed__timeline__connection"], true)
            }
            Route::Keyword(query) => {
                let session_id = gtk::glib::uuid_string_random().to_string();
                let v = self.graphql("37324993597144881", json!({"query":query,"search_session_id":session_id,"serp_session_id":session_id}), false).await?;
                let edges = n::array(&v["data"]["xdt_fbsearch__top_serp_graphql"]["edges"])?;
                let mut items = Vec::new();
                for edge in edges {
                    for p in n::array(&edge["node"]["items"])? {
                        items.push(Item::Post(n::post(p)?));
                    }
                }
                Ok(Page::complete(items))
            }
            Route::Reels => {
                let v = self.graphql("28594080480217679", json!({
                    "after":cursor,"before":null,"data":{"container_module":"clips_tab_desktop_page","seen_reels":"[]"},"first":10,"last":null,
                    "__relay_internal__pv__PolarisReelsRecoDebugOverlayEnabledrelayprovider":false,
                    "__relay_internal__pv__PolarisShortDramaEnabledrelayprovider":false
                }), false).await?;
                n::posts_connection(&v["data"]["xdt_api__v1__clips__home__connection_v2"], true)
            }
            Route::Profile(user) => {
                if cursor.is_some() {
                    return Err(Error::Unsupported);
                }
                let v = self.graphql("38154989454116081", json!({"username":user.username,
                    "data":{"count":12,"include_reel_media_seen_timestamp":true,"include_relationship_info":true,"latest_besties_reel_media":true,"latest_reel_media":true},
                    "__relay_internal__pv__PolarisMultiCaptionCarouselEnabledrelayprovider":true,
                    "__relay_internal__pv__PolarisShortDramaEnabledrelayprovider":false,
                    "__relay_internal__pv__PolarisReelsRecoDebugOverlayEnabledrelayprovider":false
                }), false).await?;
                n::posts_connection(
                    &v["data"]["xdt_api__v1__feed__user_timeline_graphql_connection"],
                    false,
                )
            }
            Route::Search(query) => {
                let v = self
                    .get(
                        "web/search/topsearch/",
                        &[("context", "blended".into()), ("query", query)],
                    )
                    .await?;
                Ok(Page::complete(
                    n::array(&v["users"])?
                        .iter()
                        .map(|x| n::user(&x["user"]).map(Item::User))
                        .collect::<Result<_, _>>()?,
                ))
            }
            Route::Followers(user, following) => {
                // Addendum explicitly says the continuation cursor was not observed.
                validate_id(&user.id)?;
                let edge = if following { "following" } else { "followers" };
                let v = self
                    .get(
                        &format!("friendships/{}/{edge}/", user.id),
                        &[
                            ("count", "12".into()),
                            ("search_surface", "follow_list_page".into()),
                        ],
                    )
                    .await?;
                let mut page = Page::complete(
                    n::array(&v["users"])?
                        .iter()
                        .map(|x| n::user(x).map(Item::User))
                        .collect::<Result<_, _>>()?,
                );
                page.continuation_unavailable = v["has_more"].as_bool().unwrap_or(false);
                Ok(page)
            }
            Route::Comments(post) => {
                let mut vars = json!({"media_id":post.id,"__relay_internal__pv__PolarisIsLoggedInrelayprovider":true});
                let id = if let Some(c) = cursor {
                    vars["after"] = json!(c);
                    vars["before"] = json!(null);
                    vars["first"] = json!(12);
                    vars["last"] = json!(null);
                    vars["sort_order"] = json!("popular");
                    "28169471862682868"
                } else {
                    "28319576384320582"
                };
                let v = self.graphql(id, vars, false).await?;
                let connection = &v["data"]["xdt_api__v1__media__media_id__comments__connection"];
                let items = n::array(&connection["edges"])?
                    .iter()
                    .map(|e| n::comment(&e["node"]).map(Item::Comment))
                    .collect::<Result<_, _>>()?;
                let next = if connection["page_info"]["has_next_page"] == true {
                    Some(n::required(&connection["page_info"]["end_cursor"])?)
                } else {
                    None
                };
                Ok(Page {
                    items,
                    next,
                    continuation_unavailable: false,
                })
            }
            Route::Inbox => {
                let params: [(&str, String); 3] = [
                    ("limit", "20".into()),
                    ("persistentBadging", "true".into()),
                    ("folder", "".into()),
                ];
                let (inbox, mailbox) =
                    tokio::join!(self.get("direct_v2/inbox/", &params), self.dm_mailbox());
                let v = inbox?;
                let mut page = Page::complete(
                    n::array(&v["inbox"]["threads"])?
                        .iter()
                        .map(|x| n::conversation(x).map(Item::Conversation))
                        .collect::<Result<_, _>>()?,
                );
                if let Ok(mailbox) = mailbox {
                    let viewer = mailbox["id"].as_str();
                    for item in &mut page.items {
                        if let Item::Conversation(thread) = item
                            && let Some(node) = mailbox_thread(&mailbox, &thread.id)
                        {
                            thread.thread_fbid = node_fbid(node);
                            thread.unread =
                                thread.unread || thread_unread(node, viewer, &self.account_id);
                        }
                    }
                }
                page.continuation_unavailable = true; // No inbox pagination builder in reference.
                Ok(page)
            }
            Route::Thread(thread) => {
                validate_id(&thread.id)?;
                let path = format!("direct_v2/threads/{}/", thread.id);
                let params: [(&str, String); 1] = [("limit", "20".into())];
                let (history, mailbox, slide) = tokio::join!(
                    self.get(&path, &params),
                    self.dm_mailbox(),
                    self.thread_slide(thread.thread_fbid.as_deref())
                );
                let v = history?;
                let mut messages = n::array(&v["thread"]["items"])?
                    .iter()
                    .map(n::message)
                    .collect::<Result<Vec<_>, _>>()?;
                messages.sort_by_key(|m| m.timestamp);
                let mut mailbox_fbid = None;
                if let Ok(mailbox) = mailbox
                    && let Some(node) = mailbox_thread(&mailbox, &thread.id)
                {
                    enrich_messages(&mut messages, node, &self.account_id);
                    merge_slide_content(&mut messages, node);
                    mailbox_fbid = node_fbid(node);
                }
                let slide = match slide {
                    // The web thread id may only resolve once the mailbox lands.
                    Err(error) if thread.thread_fbid.is_none() => match mailbox_fbid.as_deref() {
                        Some(fbid) => self.thread_slide(Some(fbid)).await,
                        None => Err(error),
                    },
                    other => other,
                };
                if let Ok(slide) = slide {
                    merge_slide_content(&mut messages, &slide);
                }
                let mut page = Page::complete(messages.into_iter().map(Item::Message).collect());
                page.continuation_unavailable = true;
                Ok(page)
            }
            Route::Stories => {
                let v = self.graphql("27703822975903310", json!({"data":{"is_following_feed":false,"reason":"web_home"},"suggestedUsersData":{"max_id":"","max_number_to_display":0,"module":"discover_people","paginate":false}}), false).await?;
                let items = n::array(&v["data"]["xdt_api__v1__feed__reels_tray"]["tray"])?
                    .iter()
                    .map(|v| {
                        Ok(Item::Story(Story {
                            id: n::required(&v["id"])?,
                            author: n::user(&v["user"])?,
                            seen: v["seen"].as_i64().unwrap_or(0)
                                >= v["latest_reel_media"].as_i64().unwrap_or(1),
                        }))
                    })
                    .collect::<Result<_, Error>>()?;
                Ok(Page::complete(items))
            }
            Route::Story(story) => {
                let v = self.graphql("37966272656351382",json!({"is_highlight":false,"media_id":null,"reel_ids_arr":[story.id],"__relay_internal__pv__PolarisCommunityNoteStoriesLabelEnabledrelayprovider":false}),false).await?;
                let items = n::array(
                    &v["data"]["xdt_api__v1__feed__reels_media"]["reels_media"][0]["items"],
                )?;
                let posts = items
                    .iter()
                    .map(|v| {
                        Ok(Item::Post(Post {
                            id: n::required(&v["pk"])?,
                            code: String::new(),
                            author: story.author.clone(),
                            caption: String::new(),
                            media: vec![n::media(v)],
                            liked: v["has_liked"].as_bool().unwrap_or(false),
                            likes: 0,
                            comments: 0,
                            timestamp: v["taken_at"].as_i64().unwrap_or(0),
                        }))
                    })
                    .collect::<Result<_, Error>>()?;
                Ok(Page::complete(posts))
            }
            Route::Notifications => {
                let v = self
                    .graphql(
                        "28455660644038251",
                        json!({"inbox_request_data":{},"pending_request_data":{}}),
                        false,
                    )
                    .await?;
                let inbox = &v["data"]["xdt_activity_inbox"];
                let mut items = Vec::new();
                for key in ["new_stories", "old_stories"] {
                    if key == "new_stories" && inbox.get(key).is_none() {
                        continue;
                    }
                    for item in n::array(&inbox[key])? {
                        items.push(Item::Notification(Notification {
                            id: n::required(&item["pk"])?,
                            text: n::string(&item["args"]["text"]),
                            user: n::user(&item["args"]["users"][0]).ok(),
                        }));
                    }
                }
                Ok(Page::complete(items))
            }
        }
    }
    pub async fn like(&self, target: &LikeTarget, liked: bool) -> Result<bool, Error> {
        // The authenticated page supplies the viewer's interop ID. Inbox
        // availability must not determine whether media actions can run.
        let kind = match target {
            LikeTarget::Post(_) => "post/reel",
            LikeTarget::Comment(_) => "comment",
            LikeTarget::Story(_) => "story",
        };
        tracing::info!(target = kind, liked, "Like action requested");
        let context = self.web_context().await.map_err(|error| {
            tracing::warn!(%error, "Like failed while loading authenticated web context");
            error
        })?;
        let actor_id = context.actor_id().map_err(|error| {
            tracing::warn!(%error, "Like failed: authenticated page has no valid mutation actor ID");
            error
        })?;
        let media_id = match target {
            LikeTarget::Post(id) => id,
            LikeTarget::Comment(comment_id) => {
                let (id, key, typename) = if liked {
                    (
                        "27184292767848867",
                        "xig_comment_like",
                        "XIGCommentLikeMutationResponse",
                    )
                } else {
                    (
                        "27318337671093716",
                        "xig_comment_unlike",
                        "XIGCommentUnlikeMutationResponse",
                    )
                };
                let v = self
                    .graphql(
                        id,
                        json!({"input": {
                            "comment_id":comment_id,"actor_id":actor_id,
                            "client_mutation_id":gtk::glib::uuid_string_random().to_string()
                        }}),
                        true,
                    )
                    .await?;
                return confirmed_like(&v, key, typename, liked);
            }
            LikeTarget::Story(media_id) => {
                let (id, key, typename) = if liked {
                    (
                        "26938887309082050",
                        "xig_send_story_like",
                        "XIGSendStoryLikeResponsePayload",
                    )
                } else {
                    (
                        "26510485515280697",
                        "xig_unsend_story_like",
                        "XIGUnsendStoryLikeResponsePayload",
                    )
                };
                let v = self
                    .graphql(
                        id,
                        json!({"input": {
                            "media_id":media_id,"actor_id":actor_id,
                            "client_mutation_id":gtk::glib::uuid_string_random().to_string()
                        }}),
                        true,
                    )
                    .await?;
                return confirmed_like(&v, key, typename, liked);
            }
        };
        let (id, key) = if liked {
            ("27182485238052618", "xig_media_like")
        } else {
            ("27345296031770102", "xig_media_unlike")
        };
        let mut input = json!({"actor_id":actor_id,"client_mutation_id":gtk::glib::uuid_string_random().to_string(),"media_id":media_id,"tracking_token":null});
        if liked {
            input["container_module"] = json!("single_post");
        }
        let v = self.graphql(id, json!({"input":input}), true).await?;
        let state = v["data"][key]["media"]["has_liked"]
            .as_bool()
            .ok_or_else(|| {
                tracing::warn!(key, response = %super::diagnostics::shape(&v), "Like response is missing media.has_liked");
                Error::Protocol
            })?;
        if state != liked {
            tracing::warn!(
                expected = liked,
                actual = state,
                "Viewfinder did not apply the requested like state"
            );
            return Err(Error::Protocol);
        }
        Ok(state)
    }
    pub async fn follow(&self, user: &User, follow: bool) -> Result<Relationship, Error> {
        let (id, key) = if follow {
            ("27767812149509802", "xdt_create_friendship")
        } else {
            ("25174972798866458", "xdt_destroy_friendship")
        };
        let v = self
            .graphql(
                id,
                json!({"target_user_id":user.id,"data":{"include_follow_friction_check":true}}),
                true,
            )
            .await?;
        let relationship = n::relationship(&v["data"][key]["friendship_status"]);
        if relationship == Relationship::Unknown {
            Err(Error::Protocol)
        } else {
            Ok(relationship)
        }
    }
    pub async fn comment(&self, post: &Post, text: &str) -> Result<Comment, Error> {
        let v = self.graphql("27261905640092552",json!({"data":{"comment_text":text,"media_id":post.id,"replied_to_comment_id":null,"tracking_token":null}}),true).await?;
        n::comment(&v["data"]["xig_comment_create"]["comment_dict"])
    }
    pub async fn mark_story_seen(&self, story: &Story, post: &Post) -> Result<(), Error> {
        for id in [&story.id, &post.id, &story.author.id] {
            validate_id(id)?;
        }
        let seen_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let v = self
            .graphql(
                "36133297086317984",
                json!({
                    "reelId": story.id,
                    "reelMediaId": post.id,
                    "reelMediaOwnerId": story.author.id,
                    "reelMediaTakenAt": post.timestamp,
                    "viewSeenAt": seen_at
                }),
                true,
            )
            .await?;
        if v["data"]["xdt_mark_story_reel_seen"]["__typename"].is_string() {
            Ok(())
        } else {
            tracing::warn!(response = %super::diagnostics::shape(&v), "Story seen response did not confirm the mutation");
            Err(Error::Protocol)
        }
    }
    /// The supplied reference has no send receipt schema. A successful transport
    /// response is not represented as confirmed delivery. UI must reconcile history.
    pub async fn send_message(
        &self,
        thread: &Conversation,
        text: &str,
        context: &str,
        reply_to: Option<&Message>,
    ) -> Result<Option<Page>, Error> {
        validate_id(&thread.id)?;
        let mut target = thread.clone();
        if target.thread_fbid.is_none() {
            let mailbox = self.dm_mailbox().await?;
            target.thread_fbid = mailbox_thread(&mailbox, &thread.id).and_then(node_fbid);
        }
        // Select the legacy envelope only when no web ID is available, never
        // retry a potentially accepted mutation using a different envelope.
        let result = self
            .graphql(
                "26911679871773184",
                send_variables(&target, text, context, reply_to),
                true,
            )
            .await;
        // Reconcile even after a transport error: the server may have accepted it.
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            match self.load(Route::Thread(target.clone()), None).await {
                Ok(page) if confirms_send(&page, &self.account_id, context) => {
                    return Ok(Some(page));
                }
                Ok(_) => (),
                Err(_) => break,
            }
        }
        result?;
        Ok(None)
    }

    /// Emoji reactions broadcast over REST `broadcast/reaction/` and confirm by
    /// reloading history: a transport success without the reaction on the item
    /// is not a completed reaction.
    pub async fn react_message(
        &self,
        thread: &Conversation,
        message: &Message,
        emoji: &str,
    ) -> Result<Page, Error> {
        validate_id(&thread.id)?;
        validate_id(&message.id)?;
        let create = !message
            .reactions
            .iter()
            .any(|r| r.sender == self.account_id && r.emoji == emoji);
        let token = gtk::glib::uuid_string_random().to_string();
        let mut fields = broadcast_fields(&thread.id, &token);
        fields.extend([
            ("item_type".into(), "reaction".into()),
            ("reaction_type".into(), "like".into()),
            (
                "reaction_status".into(),
                if create { "created" } else { "deleted" }.into(),
            ),
            ("node_type".into(), "item".into()),
            ("item_id".into(), message.id.clone()),
            ("emoji".into(), emoji.to_owned()),
            ("send_attribution".into(), "message_reaction".into()),
            ("reaction_action_source".into(), "double_tap".into()),
        ]);
        if let Some(device) = &self.device_id {
            fields.push(("device_id".into(), device.clone()));
        }
        if let Some(context) = &message.client_context {
            fields.push(("original_message_client_context".into(), context.clone()));
        }
        let response = self
            .execute(
                self.http
                    .post(format!(
                        "{}/api/v1/direct_v2/threads/broadcast/reaction/",
                        super::BASE
                    ))
                    .form(&fields),
            )
            .await?;
        if response["status"] != "ok" {
            return Err(Error::Protocol);
        }
        self.load(Route::Thread(thread.clone()), None).await
    }

    /// Tray stickers send as a generic share referencing the sticker id. The
    /// reference documents the tray query but no send body, so delivery is
    /// reconciled from history exactly like text sends.
    pub async fn send_sticker(
        &self,
        thread: &Conversation,
        sticker: &Sticker,
        context: &str,
    ) -> Result<Option<Page>, Error> {
        validate_id(&thread.id)?;
        if sticker.id.is_empty() {
            return Err(Error::Protocol);
        }
        let mut params = serde_json::json!({"sticker_id": sticker.id});
        if let Some(ent) = sticker
            .ent_type
            .as_deref()
            .and_then(|t| t.parse::<i64>().ok())
        {
            params["embedded_ent_type"] = serde_json::json!(ent);
        }
        let mut fields = broadcast_fields(&thread.id, context);
        fields.push(("json_params".into(), params.to_string()));
        if let Some(device) = &self.device_id {
            fields.push(("device_id".into(), device.clone()));
        }
        let result = self
            .execute(
                self.http
                    .post(format!(
                        "{}/api/v1/direct_v2/threads/broadcast/generic_share/",
                        super::BASE
                    ))
                    .form(&fields),
            )
            .await;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            match self.load(Route::Thread(thread.clone()), None).await {
                Ok(page) if confirms_send(&page, &self.account_id, context) => {
                    return Ok(Some(page));
                }
                Ok(_) => (),
                Err(_) => break,
            }
        }
        result?;
        Ok(None)
    }

    /// The composer sticker tray (IGDComposerRichContentStickersV2Query).
    pub async fn stickers(&self) -> Result<Vec<StickerPack>, Error> {
        let v = self
            .graphql("27793508156903587", serde_json::json!({}), false)
            .await?;
        let sections = n::array(&v["data"]["xig_igd_stickers_query"]["sections"]["edges"])?;
        let mut packs = Vec::new();
        for edge in sections {
            let node = &edge["node"];
            let mut pack = StickerPack {
                title: n::string(&node["title"]),
                ..StickerPack::default()
            };
            let ent_type = n::optional(&node["type"]);
            if let Ok(edges) = n::array(&node["stickers"]["edges"]) {
                for sticker in edges {
                    let s = &sticker["node"];
                    let id = n::string(&s["id"]);
                    let Some(media) = n::sticker_media(&s["animated_info"]) else {
                        continue;
                    };
                    if id.is_empty() {
                        continue;
                    }
                    pack.stickers.push(Sticker {
                        id,
                        alt: n::string(&s["alt_text"]),
                        media,
                        ent_type: ent_type.clone(),
                    });
                }
            }
            if !pack.stickers.is_empty() {
                packs.push(pack);
            }
        }
        Ok(packs)
    }

    pub async fn mark_thread_read(
        &self,
        thread: &Conversation,
        message: &Message,
    ) -> Result<(), Error> {
        validate_id(&thread.id)?;
        if let Some(id) = &message.message_id {
            let metadata = json!({"ig_thread_igid": thread.id});
            let validation = self.graphql("35211594988486314", json!({
                "metadata": metadata, "data": {"message_id": id, "message_timestamp_ms": message.timestamp / 1000}
            }), true).await?;
            if validation["data"]["xig_direct_mark_read_id_validation"] != true {
                return Err(Error::Protocol);
            }
            let response = self
                .graphql(
                    "27399783383056109",
                    json!({
                        "metadata": metadata, "data": {"item_id": "", "message_id": id}
                    }),
                    true,
                )
                .await?;
            if response["data"]["xig_direct_item_seen_mutation_with_slide_messaging_response"]["watermark_timestamp_ms"].as_i64().is_none() {
                return Err(Error::Protocol);
            }
        } else {
            validate_id(&message.id)?;
            let response = self
                .execute(self.http.post(format!(
                    "{}/api/v1/direct_v2/threads/{}/items/{}/seen/",
                    super::BASE,
                    thread.id,
                    message.id
                )))
                .await?;
            if response["status"] != "ok" {
                return Err(Error::Protocol);
            }
        }
        Ok(())
    }
}

fn confirms_send(page: &Page, account: &str, context: &str) -> bool {
    let message_id = format!("mid.${context}");
    page.items.iter().any(|item| {
        matches!(item, Item::Message(m)
        if m.sender == account && (m.client_context.as_deref() == Some(context)
            || m.message_id.as_deref() == Some(message_id.as_str())))
    })
}

fn mailbox_thread<'a>(
    mailbox: &'a serde_json::Value,
    rest_id: &str,
) -> Option<&'a serde_json::Value> {
    mailbox["threads_by_folder"]["edges"]
        .as_array()?
        .iter()
        .map(|edge| &edge["node"]["as_ig_direct_thread"])
        .find(|node| node["thread_id"].as_str() == Some(rest_id))
}

/// A thread is unread when manually flagged or when its latest activity is
/// newer than the viewer's `slide_read_receipts` watermark. The viewer is
/// identified by interop fbid: the mailbox root `id`, the `users[]` entry
/// matching the account, or `viewer_id` on the node. A newest slide message
/// authored by the viewer counts as read regardless of watermark lag.
fn thread_unread(node: &serde_json::Value, viewer: Option<&str>, account_id: &str) -> bool {
    if node["marked_as_unread"].as_bool().unwrap_or(false) {
        return true;
    }
    let candidates: Vec<&str> = [
        viewer,
        node["users"].as_array().and_then(|users| {
            users
                .iter()
                .find(|u| n::string(&u["id"]) == account_id)
                .and_then(|u| u["interop_messaging_user_fbid"].as_str())
        }),
        node["viewer_id"].as_str(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if candidates.is_empty() {
        return false;
    }
    let own = |v: &serde_json::Value| v.as_str().is_some_and(|fbid| candidates.contains(&fbid));
    let newest = node["slide_messages"]["edges"]
        .as_array()
        .and_then(|edges| {
            edges
                .iter()
                .map(|e| &e["node"])
                .max_by_key(|m| m["timestamp_ms"].as_i64().unwrap_or(0))
        });
    let last_activity = node["last_activity_timestamp_ms"]
        .as_i64()
        .unwrap_or(0)
        .max(newest.and_then(|m| m["timestamp_ms"].as_i64()).unwrap_or(0));
    if last_activity <= 0 {
        return false;
    }
    if newest.is_some_and(|m| {
        own(&m["sender_fbid"]) && m["timestamp_ms"].as_i64().unwrap_or(0) >= last_activity
    }) {
        return false;
    }
    let Some(receipts) = node["slide_read_receipts"].as_array() else {
        return false;
    };
    let watermark = receipts
        .iter()
        .find(|r| own(&r["participant_fbid"]))
        .and_then(|r| r["watermark_timestamp_ms"].as_i64())
        .unwrap_or(0);
    last_activity > watermark
}

fn node_fbid(node: &serde_json::Value) -> Option<String> {
    node["thread_fbid"]
        .as_str()
        .or_else(|| node["id"].as_str())
        .map(str::to_owned)
}

fn send_variables(
    thread: &Conversation,
    text: &str,
    context: &str,
    reply_to: Option<&Message>,
) -> serde_json::Value {
    let mut vars = json!({
        "ig_thread_igid": thread.thread_fbid.as_ref().unwrap_or(&thread.id),
        "offline_threading_id": context, "text": {"sensitive_string_value": text},
        "mentions": [], "mentioned_user_ids": [], "recipient_igids": null,
        "replied_to_client_context": reply_to.and_then(|m| m.client_context.clone()),
        "replied_to_item_id": reply_to.map(|m| m.id.clone()),
        "reply_to_message_id": reply_to.and_then(|m| m.message_id.clone()),
        "sampled": null, "commands": null,
        "forwarded_from_thread_id": null, "is_forwarded_from_own_message": null,
        "send_attribution": "igd_web_chat_tab:in_thread"
    });
    if thread.thread_fbid.is_none() {
        vars["text"] = json!(text);
        vars["send_attribution"] = json!("direct_thread");
        vars["mentions"] = json!(null);
        vars["mentioned_user_ids"] = json!(null);
        json!({"data": vars})
    } else {
        vars
    }
}

/// Shared form fields for `direct_v2/threads/broadcast/` item sends. The same
/// trio of client_context, mutation_token and offline_threading_id correlates
/// the item once history is reloaded.
fn broadcast_fields(thread_id: &str, token: &str) -> Vec<(String, String)> {
    vec![
        ("action".into(), "send_item".into()),
        ("client_context".into(), token.to_owned()),
        ("mutation_token".into(), token.to_owned()),
        ("offline_threading_id".into(), token.to_owned()),
        ("thread_ids".into(), json!([thread_id]).to_string()),
        ("is_shh_mode".into(), "0".into()),
        ("send_silently".into(), "false".into()),
        ("is_x_transport_forward".into(), "false".into()),
        ("is_ae_dual_send".into(), "false".into()),
        ("btt_dual_send".into(), "false".into()),
    ]
}

fn enrich_messages(messages: &mut [Message], node: &serde_json::Value, account_id: &str) {
    for message in messages {
        if message.message_id.is_none()
            && let Some(edges) = node["slide_messages"]["edges"].as_array()
        {
            let expected = message
                .client_context
                .as_ref()
                .map(|id| format!("mid.${id}"));
            let sender_fbid = node["users"]
                .as_array()
                .and_then(|users| users.iter().find(|u| n::string(&u["id"]) == message.sender))
                .and_then(|u| u["interop_messaging_user_fbid"].as_str());
            let candidates: Vec<_> = edges
                .iter()
                .map(|e| &e["node"])
                .filter(|m| {
                    expected
                        .as_deref()
                        .is_some_and(|id| m["id"].as_str() == Some(id))
                        || (message.timestamp > 0
                            && sender_fbid.is_some()
                            && m["sender_fbid"].as_str() == sender_fbid
                            && m["timestamp_ms"].as_i64() == Some(message.timestamp / 1000))
                })
                .collect();
            if candidates.len() == 1 {
                message.message_id = candidates[0]["id"].as_str().map(str::to_owned);
            }
        }
        if message.sender != account_id || message.timestamp <= 0 {
            continue;
        }
        if let Some(receipts) = node["slide_read_receipts"].as_array() {
            for receipt in receipts {
                if receipt["watermark_timestamp_ms"].as_i64().unwrap_or(0)
                    < message.timestamp / 1000
                {
                    continue;
                }
                let participant = receipt["participant_fbid"].as_str();
                if let Some(user) = node["users"].as_array().and_then(|users| {
                    users.iter().find(|user| {
                        participant.is_some()
                            && user["interop_messaging_user_fbid"].as_str() == participant
                            && n::string(&user["id"]) != account_id
                    })
                }) {
                    message.seen_by.push(n::string(&user["username"]));
                }
            }
        }
    }
}

/// `direct_v2` masks newer item types (cutout stickers, some shares) as
/// `item_type: "placeholder"`, which normalizes to `Attachment::Unavailable`.
/// The slide channel carries the real payload under `content`, keyed by the
/// same `mid.$` id. Merge it so masked items still render.
fn merge_slide_content(messages: &mut [Message], node: &serde_json::Value) {
    let Some(edges) = node["slide_messages"]["edges"].as_array() else {
        return;
    };
    let slides: Vec<&serde_json::Value> = edges.iter().map(|e| &e["node"]).collect();
    let sender_fbid = |sender: &str| {
        node["users"].as_array().and_then(|users| {
            users
                .iter()
                .find(|u| n::string(&u["id"]) == sender)
                .and_then(|u| u["interop_messaging_user_fbid"].as_str())
        })
    };
    for message in messages.iter_mut() {
        if !message
            .attachments
            .iter()
            .all(|a| matches!(a, Attachment::Unavailable))
        {
            continue;
        }
        let Some(slide) = slides.iter().find(|m| {
            (message.message_id.is_some() && m["id"].as_str() == message.message_id.as_deref())
                || (message.timestamp > 0
                    && sender_fbid(&message.sender)
                        .is_some_and(|fbid| m["sender_fbid"].as_str() == Some(fbid))
                    && m["timestamp_ms"].as_i64() == Some(message.timestamp / 1000))
        }) else {
            continue;
        };
        let content = &slide["content"];
        match content["__typename"].as_str() {
            Some("SlideMessageCutoutStickerXMAContent") => {
                if let Some(url) = n::optional(&content["preview_url"]) {
                    message.attachments = vec![Attachment::Animated {
                        media: Media {
                            thumbnail: Some(url.clone()),
                            image: Some(url),
                            video: None,
                            width: content["preview_width"].as_i64().unwrap_or(200) as i32,
                            height: content["preview_height"].as_i64().unwrap_or(200) as i32,
                        },
                        alt: n::optional(&content["alt_text"]).unwrap_or_else(|| "Sticker".into()),
                    }];
                }
            }
            Some("SlideMessageAnimatedMediaContent") => {
                if let Some(am) = content["animated_media"].as_array().and_then(|a| a.first()) {
                    let image = n::optional(&am["attachment_webp_url"])
                        .or_else(|| n::optional(&am["preview_cdn_url"]));
                    let video = n::optional(&am["attachment_mp4_url"]);
                    if image.is_some() || video.is_some() {
                        message.attachments = vec![Attachment::Animated {
                            media: Media {
                                thumbnail: image.clone(),
                                image,
                                video,
                                width: am["preview_width"].as_i64().unwrap_or(200) as i32,
                                height: am["preview_height"].as_i64().unwrap_or(200) as i32,
                            },
                            alt: n::optional(&am["alt_text"]).unwrap_or_else(|| "Sticker".into()),
                        }];
                    }
                }
            }
            Some("SlideMessageImageContent") => {
                let media: Vec<Attachment> = content["attachments"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                let url = n::optional(&item["attachment_cdn_url"])
                                    .or_else(|| n::optional(&item["preview_cdn_url"]))?;
                                Some(Attachment::Media(Media {
                                    thumbnail: n::optional(&item["preview_cdn_url"])
                                        .or_else(|| Some(url.clone())),
                                    image: Some(url),
                                    video: None,
                                    width: item["preview_width"].as_i64().unwrap_or(0) as i32,
                                    height: item["preview_height"].as_i64().unwrap_or(0) as i32,
                                }))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !media.is_empty() {
                    message.attachments = media;
                }
            }
            Some("SlideMessageXMAContent") => {
                let xma = &content["xma"];
                if let Some(url) = n::optional(&xma["preview_image"]["url"])
                    .or_else(|| n::optional(&xma["xmaPreviewImage"]["url"]))
                {
                    let target = n::string(&xma["target_url"]);
                    message.attachments = vec![Attachment::Post(Box::new(Post {
                        id: n::string(&xma["target_id"]),
                        code: target
                            .split('/')
                            .filter(|part| !part.is_empty())
                            .nth(1)
                            .unwrap_or_default()
                            .to_owned(),
                        author: User {
                            username: n::string(&xma["header_title_text"]),
                            avatar: n::optional(&xma["header_icon"]["url"]),
                            ..User::default()
                        },
                        caption: String::new(),
                        media: vec![Media {
                            thumbnail: Some(url.clone()),
                            image: Some(url),
                            video: None,
                            width: xma["preview_image"]["width"].as_i64().unwrap_or(0) as i32,
                            height: xma["preview_image"]["height"].as_i64().unwrap_or(0) as i32,
                        }],
                        liked: false,
                        likes: 0,
                        comments: 0,
                        timestamp: 0,
                    }))];
                }
            }
            Some("SlideMessageAdminText") if message.text.is_empty() => {
                let text = content["text_fragments"]
                    .as_array()
                    .map(|fragments| {
                        fragments
                            .iter()
                            .filter_map(|f| f["plaintext"].as_str())
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                if !text.is_empty() {
                    message.text = text;
                    message.attachments = vec![];
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod like_tests {
    use super::*;
    #[test]
    fn dm_payload_keeps_rest_and_web_identifiers_distinct() {
        let mut thread = Conversation {
            id: "34028236".into(),
            thread_fbid: Some("987".into()),
            ..Conversation::default()
        };
        let vars = send_variables(&thread, "Hello\n🌻", "123", None);
        assert_eq!(vars["ig_thread_igid"], "987");
        assert_eq!(vars["text"]["sensitive_string_value"], "Hello\n🌻");
        assert_eq!(vars["send_attribution"], "igd_web_chat_tab:in_thread");
        assert!(vars.get("data").is_none());
        assert!(vars["replied_to_item_id"].is_null());
        let original = Message {
            id: "30123".into(),
            message_id: Some("mid.$abc".into()),
            client_context: Some("ctx-1".into()),
            ..Message::default()
        };
        let replied = send_variables(&thread, "Back", "124", Some(&original));
        assert_eq!(replied["replied_to_item_id"], "30123");
        assert_eq!(replied["replied_to_client_context"], "ctx-1");
        assert_eq!(replied["reply_to_message_id"], "mid.$abc");
        thread.thread_fbid = None;
        let legacy = send_variables(&thread, "Hello", "123", Some(&original));
        assert_eq!(legacy["data"]["ig_thread_igid"], "34028236");
        assert_eq!(legacy["data"]["text"], "Hello");
        assert_eq!(legacy["data"]["replied_to_item_id"], "30123");
    }

    #[test]
    fn broadcast_forms_carry_correlation_tokens_and_targets() {
        let fields = broadcast_fields("34028236", "token-1");
        let get = |key: &str| {
            fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("action"), Some("send_item"));
        assert_eq!(get("client_context"), Some("token-1"));
        assert_eq!(get("mutation_token"), Some("token-1"));
        assert_eq!(get("offline_threading_id"), Some("token-1"));
        assert_eq!(get("thread_ids"), Some("[\"34028236\"]"));
    }

    #[test]
    fn dm_confirmation_requires_exact_context_and_own_sender() {
        let mut message = Message {
            sender: "me".into(),
            text: "hi".into(),
            ..Message::default()
        };
        assert!(!confirms_send(
            &Page::complete(vec![Item::Message(message.clone())]),
            "me",
            "123"
        ));
        message.client_context = Some("123".into());
        assert!(confirms_send(
            &Page::complete(vec![Item::Message(message.clone())]),
            "me",
            "123"
        ));
        assert!(!confirms_send(
            &Page::complete(vec![Item::Message(message.clone())]),
            "other",
            "123"
        ));
        message.client_context = None;
        message.message_id = Some("mid.$123".into());
        assert!(confirms_send(
            &Page::complete(vec![Item::Message(message)]),
            "me",
            "123"
        ));
    }

    #[test]
    fn dm_receipts_map_fbid_to_people_and_compare_milliseconds() {
        let node = json!({"users": [
            {"id":"me", "username":"myself", "interop_messaging_user_fbid":"10"},
            {"id":"them", "username":"alex", "interop_messaging_user_fbid":"20"}],
            "slide_read_receipts": [
                {"participant_fbid":"10", "watermark_timestamp_ms":9000},
                {"participant_fbid":"20", "watermark_timestamp_ms":2000},
                {"participant_fbid":"unknown", "watermark_timestamp_ms":9000}]});
        let mut messages = vec![
            Message {
                sender: "me".into(),
                timestamp: 2000000,
                ..Message::default()
            },
            Message {
                sender: "me".into(),
                timestamp: 3000000,
                ..Message::default()
            },
            Message {
                sender: "them".into(),
                timestamp: 1000000,
                ..Message::default()
            },
        ];
        enrich_messages(&mut messages, &node, "me");
        assert_eq!(messages[0].seen_by, ["alex"]);
        assert!(messages[1].seen_by.is_empty());
        assert!(messages[2].seen_by.is_empty());
        let mailbox = json!({"threads_by_folder":{"edges":[{"node":{"as_ig_direct_thread":{
            "id":"fbid", "thread_key":"url", "thread_id":"rest"
        }}}]}});
        assert_eq!(
            node_fbid(mailbox_thread(&mailbox, "rest").unwrap()).as_deref(),
            Some("fbid")
        );
        assert!(mailbox_thread(&mailbox, "url").is_none());
    }

    #[test]
    fn inbox_unread_uses_viewer_watermark_and_own_latest_message() {
        let node = json!({
            "thread_id": "rest",
            "last_activity_timestamp_ms": 10000,
            "users": [
                {"id":"me","username":"myself","interop_messaging_user_fbid":"10"},
                {"id":"them","username":"alex","interop_messaging_user_fbid":"20"}
            ],
            "slide_messages": {"edges": [
                {"node": {"sender_fbid":"20","timestamp_ms":9000}},
                {"node": {"sender_fbid":"20","timestamp_ms":10000}}
            ]},
            "slide_read_receipts": [
                {"participant_fbid":"10","watermark_timestamp_ms":10000},
                {"participant_fbid":"20","watermark_timestamp_ms":10000}
            ]
        });
        assert!(!thread_unread(&node, Some("10"), "me"));
        let mut stale = node.clone();
        stale["slide_read_receipts"][0]["watermark_timestamp_ms"] = json!(9000);
        assert!(thread_unread(&stale, Some("10"), "me"));
        assert!(thread_unread(&stale, None, "me"));
        stale["marked_as_unread"] = json!(true);
        assert!(thread_unread(&stale, Some("10"), "me"));
        let mut own = node.clone();
        own["slide_read_receipts"][0]["watermark_timestamp_ms"] = json!(9000);
        own["slide_messages"]["edges"][1]["node"]["sender_fbid"] = json!("10");
        assert!(!thread_unread(&own, Some("10"), "me"));
        assert!(!thread_unread(&node, None, "stranger"));
    }

    #[test]
    fn comment_and_story_success_requires_the_expected_payload() {
        for (key, name, liked) in [
            ("xig_comment_like", "XIGCommentLikeMutationResponse", true),
            (
                "xig_comment_unlike",
                "XIGCommentUnlikeMutationResponse",
                false,
            ),
            (
                "xig_send_story_like",
                "XIGSendStoryLikeResponsePayload",
                true,
            ),
            (
                "xig_unsend_story_like",
                "XIGUnsendStoryLikeResponsePayload",
                false,
            ),
        ] {
            assert_eq!(
                confirmed_like(&json!({"data":{key:{"__typename":name}}}), key, name, liked)
                    .unwrap(),
                liked
            );
            for bad in [
                json!({}),
                json!({"data":{key:null}}),
                json!({"data":{key:{"__typename":"Unexpected"}}}),
            ] {
                assert!(confirmed_like(&bad, key, name, liked).is_err());
            }
        }
    }
    #[test]
    fn comment_likes_accept_both_documented_field_names() {
        for fields in [
            json!({"has_liked_comment":true,"comment_like_count":12}),
            json!({"has_liked":true,"like_count":12}),
        ] {
            let mut v =
                json!({"pk":"123", "user":{"pk":"42","username":"fixture"}, "text":"hello"});
            v.as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let c = n::comment(&v).unwrap();
            assert!(c.liked);
            assert_eq!(c.likes, 12);
        }
    }

    /// REST `placeholder` items carry the slide `mid.$` id; slide `content`
    /// restores the masked attachment.
    #[test]
    fn slide_content_replaces_masked_placeholder_attachments() {
        let node = json!({
            "users": [{"id":"them","username":"alex","interop_messaging_user_fbid":"20"}],
            "slide_messages": {"edges": [
                {"node": {"id":"mid.$cut", "sender_fbid":"20", "timestamp_ms":9000,
                    "content": {"__typename":"SlideMessageCutoutStickerXMAContent",
                        "preview_url":"https://scontent-det1-1.cdninstagram.com/v/sticker.png",
                        "preview_width":878, "preview_height":1220, "alt_text":null}}},
                {"node": {"id":"mid.$gif", "sender_fbid":"20", "timestamp_ms":8000,
                    "content": {"__typename":"SlideMessageAnimatedMediaContent",
                        "animated_media": [{"attachment_webp_url":"https://external-det1-1.xx.fbcdn.net/v/a.webp",
                            "attachment_mp4_url":"https://external-det1-1.xx.fbcdn.net/v/a.mp4",
                            "preview_cdn_url":"https://external-det1-1.xx.fbcdn.net/v/p.webp",
                            "preview_width":211, "preview_height":200, "is_sticker":true}]}}},
                {"node": {"id":"mid.$admin", "sender_fbid":"20", "timestamp_ms":7000,
                    "content": {"__typename":"SlideMessageAdminText",
                        "text_fragments":[{"plaintext":"Liked a message"}]}}}
            ]}
        });
        let mut messages = vec![
            Message {
                message_id: Some("mid.$cut".into()),
                sender: "them".into(),
                timestamp: 9000000,
                attachments: vec![Attachment::Unavailable],
                ..Message::default()
            },
            Message {
                // No message_id: falls back to sender + timestamp matching.
                sender: "them".into(),
                timestamp: 8000000,
                attachments: vec![Attachment::Unavailable],
                ..Message::default()
            },
            Message {
                message_id: Some("mid.$admin".into()),
                sender: "them".into(),
                timestamp: 7000000,
                attachments: vec![Attachment::Unavailable],
                ..Message::default()
            },
            Message {
                message_id: Some("mid.$unknown".into()),
                sender: "them".into(),
                timestamp: 6000000,
                attachments: vec![Attachment::Unavailable],
                ..Message::default()
            },
            Message {
                message_id: Some("mid.$text".into()),
                sender: "them".into(),
                timestamp: 5000000,
                text: "keep me".into(),
                ..Message::default()
            },
        ];
        merge_slide_content(&mut messages, &node);
        let Attachment::Animated { media, alt } = &messages[0].attachments[0] else {
            panic!("cutout sticker should merge as animated attachment")
        };
        assert_eq!(
            media.image.as_deref(),
            Some("https://scontent-det1-1.cdninstagram.com/v/sticker.png")
        );
        assert_eq!((media.width, media.height), (878, 1220));
        assert_eq!(alt, "Sticker");
        let Attachment::Animated { media, .. } = &messages[1].attachments[0] else {
            panic!("animated media should merge by sender and timestamp")
        };
        assert_eq!(
            media.video.as_deref(),
            Some("https://external-det1-1.xx.fbcdn.net/v/a.mp4")
        );
        assert_eq!(messages[2].text, "Liked a message");
        assert!(messages[2].attachments.is_empty());
        assert!(matches!(
            messages[3].attachments[0],
            Attachment::Unavailable
        ));
        assert!(messages[4].attachments.is_empty());
    }

    #[test]
    #[ignore = "live thread load against the local session"]
    fn live_thread_slide_merge() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let session = crate::viewfinder::session::Session::restore()
                .await
                .unwrap_or_else(|_| {
                    crate::viewfinder::session::Session::parse(
                        &std::fs::read("session_cookies.json").unwrap(),
                    )
                    .unwrap()
                });
            let client = Client::new(session).unwrap();
            let thread = Conversation {
                id: std::env::var("VF_TEST_THREAD")
                    .unwrap_or_else(|_| "340282366841710301244260004355658495848".into()),
                ..Conversation::default()
            };
            let page = client.load(Route::Thread(thread), None).await.unwrap();
            for item in &page.items {
                if let Item::Message(m) = item {
                    eprintln!(
                        "{} | text={:?} | attachments={:?}",
                        m.id,
                        m.text,
                        m.attachments
                            .iter()
                            .map(|a| match a {
                                Attachment::Unavailable => "unavailable".to_owned(),
                                Attachment::Animated { .. } => "animated".to_owned(),
                                Attachment::Media(_) => "media".to_owned(),
                                Attachment::Post(_) => "post".to_owned(),
                                Attachment::Link { .. } => "link".to_owned(),
                                Attachment::Voice { .. } => "voice".to_owned(),
                            })
                            .collect::<Vec<_>>()
                    );
                }
            }
        });
    }
}
