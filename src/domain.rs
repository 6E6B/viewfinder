//! Stable application models. No transport keys or GTK objects live here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct User {
    pub id: String,
    pub username: String,
    pub name: String,
    pub avatar: Option<String>,
    pub biography: String,
    pub followers: u64,
    pub following_count: u64,
    pub posts: u64,
    pub relationship: Relationship,
    pub private: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Relationship {
    Following,
    Requested,
    NotFollowing,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    pub thumbnail: Option<String>,
    pub image: Option<String>,
    pub video: Option<String>,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Post {
    pub id: String,
    pub code: String,
    pub author: User,
    pub caption: String,
    pub media: Vec<Media>,
    pub liked: bool,
    pub likes: u64,
    pub comments: u64,
    pub timestamp: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comment {
    pub id: String,
    pub author: User,
    pub text: String,
    pub liked: bool,
    pub likes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LikeTarget {
    Post(String),
    Comment(String),
    Story(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conversation {
    pub id: String,
    pub thread_fbid: Option<String>,
    pub unread: bool,
    pub title: String,
    pub participants: Vec<User>,
    pub preview: String,
    pub preview_sender: String,
    pub preview_timestamp: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Attachment {
    Media(Media),
    Post(Box<Post>),
    Link { title: String, url: String },
    Unavailable,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Message {
    pub id: String,
    pub message_id: Option<String>,
    pub client_context: Option<String>,
    pub seen_by: Vec<String>,
    pub sender: String,
    pub text: String,
    pub timestamp: i64,
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Story {
    pub id: String,
    pub author: User,
    pub seen: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub id: String,
    pub text: String,
    pub user: Option<User>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Post(Post),
    User(User),
    Comment(Comment),
    Conversation(Conversation),
    Message(Message),
    Story(Story),
    Notification(Notification),
}

impl Item {
    pub fn id(&self) -> &str {
        match self {
            Self::Post(x) => &x.id,
            Self::User(x) => &x.id,
            Self::Comment(x) => &x.id,
            Self::Conversation(x) => &x.id,
            Self::Message(x) => &x.id,
            Self::Story(x) => &x.id,
            Self::Notification(x) => &x.id,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub items: Vec<Item>,
    pub next: Option<String>,
    /// Server reports more data but the reference does not document its request.
    pub continuation_unavailable: bool,
}

impl Page {
    pub fn complete(items: Vec<Item>) -> Self {
        Self {
            items,
            next: None,
            continuation_unavailable: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Home,
    Keyword(String),
    Reels,
    Inbox,
    Thread(Conversation),
    Profile(User),
    Followers(User, bool),
    Search(String),
    Comments(Post),
    Stories,
    Story(Story),
    Notifications,
}

impl Route {
    pub fn title(&self) -> String {
        match self {
            Self::Home => "Home".into(),
            Self::Keyword(q) | Self::Search(q) => q.clone(),
            Self::Reels => "Reels".into(),
            Self::Inbox => "Messages".into(),
            Self::Thread(t) => t.title.clone(),
            Self::Profile(u) => u.username.clone(),
            Self::Followers(_, following) => {
                if *following { "Following" } else { "Followers" }.into()
            }
            Self::Comments(_) => "Comments".into(),
            Self::Stories => "Stories".into(),
            Self::Story(s) => s.author.username.clone(),
            Self::Notifications => "Notifications".into(),
        }
    }
    pub fn grid(&self) -> bool {
        matches!(self, Self::Keyword(_) | Self::Reels | Self::Profile(_))
    }
}
