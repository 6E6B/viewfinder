//! Identifiers and variables transcribed from the supplied Python reference and addenda.
//! No browser probing or automatically retried mutations.
use super::{Client, Error, normalize as n, validate_id};
use crate::domain::*;
use serde_json::json;

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
    async fn dm_mailbox(&self) -> Result<serde_json::Value, Error> {
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
                false,
            )
            .await?;
        let mailbox = &v["data"]["get_slide_mailbox_for_iris_subscription"];
        n::array(&mailbox["threads_by_folder"]["edges"])?;
        Ok(mailbox.clone())
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
                let v = self
                    .get(
                        "direct_v2/inbox/",
                        &[
                            ("limit", "20".into()),
                            ("persistentBadging", "true".into()),
                            ("folder", "".into()),
                        ],
                    )
                    .await?;
                let mut page = Page::complete(
                    n::array(&v["inbox"]["threads"])?
                        .iter()
                        .map(|x| n::conversation(x).map(Item::Conversation))
                        .collect::<Result<_, _>>()?,
                );
                if let Ok(mailbox) = self.dm_mailbox().await {
                    for item in &mut page.items {
                        if let Item::Conversation(thread) = item
                            && let Some(node) = mailbox_thread(&mailbox, &thread.id)
                        {
                            thread.thread_fbid = node_fbid(node);
                            thread.unread = node["marked_as_unread"].as_bool().unwrap_or(false);
                        }
                    }
                }
                page.continuation_unavailable = true; // No inbox pagination builder in reference.
                Ok(page)
            }
            Route::Thread(thread) => {
                validate_id(&thread.id)?;
                let v = self
                    .get(
                        &format!("direct_v2/threads/{}/", thread.id),
                        &[("limit", "20".into())],
                    )
                    .await?;
                let mut messages = n::array(&v["thread"]["items"])?
                    .iter()
                    .map(n::message)
                    .collect::<Result<Vec<_>, _>>()?;
                messages.sort_by_key(|m| m.timestamp);
                if let Ok(mailbox) = self.dm_mailbox().await
                    && let Some(node) = mailbox_thread(&mailbox, &thread.id)
                {
                    enrich_messages(&mut messages, node, &self.account_id);
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
    /// The supplied reference has no send receipt schema. A successful transport
    /// response is not represented as confirmed delivery. UI must reconcile history.
    pub async fn send_message(
        &self,
        thread: &Conversation,
        text: &str,
        context: &str,
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
                send_variables(&target, text, context),
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

fn node_fbid(node: &serde_json::Value) -> Option<String> {
    node["thread_fbid"]
        .as_str()
        .or_else(|| node["id"].as_str())
        .map(str::to_owned)
}

fn send_variables(thread: &Conversation, text: &str, context: &str) -> serde_json::Value {
    let mut vars = json!({
        "ig_thread_igid": thread.thread_fbid.as_ref().unwrap_or(&thread.id),
        "offline_threading_id": context, "text": {"sensitive_string_value": text},
        "mentions": [], "mentioned_user_ids": [], "recipient_igids": null,
        "replied_to_client_context": null, "replied_to_item_id": null,
        "reply_to_message_id": null, "sampled": null, "commands": null,
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
        let vars = send_variables(&thread, "Hello\n🌻", "123");
        assert_eq!(vars["ig_thread_igid"], "987");
        assert_eq!(vars["text"]["sensitive_string_value"], "Hello\n🌻");
        assert_eq!(vars["send_attribution"], "igd_web_chat_tab:in_thread");
        assert!(vars.get("data").is_none());
        thread.thread_fbid = None;
        let legacy = send_variables(&thread, "Hello", "123");
        assert_eq!(legacy["data"]["ig_thread_igid"], "34028236");
        assert_eq!(legacy["data"]["text"], "Hello");
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
}
