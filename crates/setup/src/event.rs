use std::path::PathBuf;

use serde_json::Value;

pub trait Event: 'static {
    type Args: 'static;
}

macro_rules! gen_event {
    // Caso 1: Eventos sin argumentos
    ($($n:ident),* $(,)?) => {
        $(
            pub struct $n;
            unsafe impl Send for $n {}
            unsafe impl Sync for $n {}
            impl Event for $n {
                type Args = ();
            }
        )*
    };

    // Caso 2: Eventos con UN solo tipo (sin tupla)
    ($($n:ident ($a:ident : $ty:ty)),* $(,)?) => {
        $(
            #[doc = concat!(" ", stringify!($a), ": ", stringify!($ty))]
            pub struct $n;
            unsafe impl Send for $n {}
            unsafe impl Sync for $n {}
            impl Event for $n {
                type Args = $ty;
            }
        )*
    };

    // Caso 3a: Eventos con UN solo campo nombrado (sin tupla)
    ($($n:ident { $a:ident : $ty:ty $(,)? }),* $(,)?) => {
        $(
            #[doc = concat!(" ", stringify!($a), ": ", stringify!($ty))]
            pub struct $n;
            unsafe impl Send for $n {}
            unsafe impl Sync for $n {}
            impl Event for $n {
                type Args = $ty;
            }
        )*
    };

    // Caso 3b: Eventos con múltiples campos nombrados (con tupla)
    ($($n:ident { $first_a:ident : $first_ty:ty, $($a:ident : $ty:ty),* $(,)? }),* $(,)?) => {
        $(
            #[doc = concat!(" ", stringify!($first_a), ": ", stringify!($first_ty),)]
            $(#[doc = concat!(" ", stringify!($a), ": ", stringify!($ty))])*
            pub struct $n;
            unsafe impl Send for $n {}
            unsafe impl Sync for $n {}
            impl Event for $n {
                type Args = ($first_ty, $($ty,)*);
            }
        )*
    };
}

gen_event!(Completed);
gen_event!(SelectionSaved(path: PathBuf), Message(msg: String), Error(e: String));
gen_event! {
    Progress {
        step: String,
        current: u64,
        total: u64,
    },
    ManifestReady {
        manifest: Value,
        path: PathBuf,
    },
    CrossRefCached {
        book_id: String,
        path: PathBuf,
    },
    BibleManifestCached {
        bible_id: String,
        path: PathBuf,
    },
    BibleBookCached {
        bible_id: String,
        book_id: String,
        path: PathBuf,
    },
}
