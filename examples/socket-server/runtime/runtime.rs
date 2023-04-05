//! Please, see [super]

use crate::{
    config::SocketServerConfig,
    frontend::{
        socket_server::SocketServer,
    },
    logic::ogre_robot::OgreRobot,
};
use std::{
    sync::Arc,
    time::{SystemTime,Duration},
    ops::DerefMut,
};
use futures::future::BoxFuture;
use log::debug;
use tokio::sync::RwLock;

/// Timeout to wait for `Option` data to be filled in -- when retrieving it
const TIMEOUT: Duration = Duration::from_secs(3);
/// Time to wait on between checks for an `Option` data to be filled in -- when retrieving it
const POLL_INTERVAL: Duration = Duration::from_micros(1000);


/// Contains data filled at runtime -- not present in the config file
pub struct Runtime {

    // environment
    //////////////

    /// this process executable's absolute path, used o determine the
    /// date the executable was generated, which is important for optimization
    /// decisions regarding the need for datasets & amalgamations to be regenerated
    pub executable_path: String,

    /// allows calling `tokio_runtime.block_on()`, `tokio_runtime.spawn()`, etc.
    /// on this to run async tasks on sync contexts, although
    /// `futures::executor::block_on()` seems to be faster
    pub tokio_runtime: Option<Arc<tokio::runtime::Runtime>>,


    // logic
    ////////

    /// The [OgreRobot] object, with all sub module instances ready to show off
    ogre_robot: Option<OgreRobot>,

    // internal task communication
    //////////////////////////////

    /// The Socket Server controller -- can be used to inquiring the running state and to request the service to shutdown
    /// -- See [SocketServer]
    socket_server: Option<SocketServer<'static>>,


}

/// Macro to create getters & setters for `Option` fields -- with timeouts and dead-lock prevention
macro_rules! impl_runtime {
    ($field_name_str:        literal,
     $field_name_ident:      ident,
     $field_type:            ty,
     $set_function_name:     ident,
     $get_function_name:     ident,
     $opt_get_function_name: ident) => {

        impl Runtime {

            /// RW-Locks `runtime`, then registers the [Runtime::$field_name_ident] -- so it may be retrieved (possibly in another thread) with [$get_function_name()]\
            ///
            /// Example:
            /// ```no_compile
            ///     Runtime::$set_function_name(&runtime, $field_name_ident).await;
            pub async fn $set_function_name(runtime: &RwLock<Self>, $field_name_ident: $field_type) {
                runtime.write().await.$field_name_ident.replace($field_name_ident);
            }

            /// Gets (or waits for up to a reasonable, hard-coded timeout) the [Runtime::$field_name_ident] -- as set (possibly in another thread or task)
            /// by [$set_function_name()] -- then pass it to `callback()` to do something useful with it while `runtime` is read-locked\
            ///
            /// Example:
            /// ```no_compile
            ///     Runtime::$get_function_name(&runtime, |$field_name, _runtime| Box::pin(async move {
            ///         $field_name.broadcast_message(&contents_for_$field_name, true).await
            ///     })).await?;
            pub async fn $get_function_name<ReturnType>
                                           (runtime:  &RwLock<Self>,
                                            callback: impl for<'r> FnOnce(&'r mut $field_type) -> BoxFuture<'r, ReturnType> + Send)
                                           -> ReturnType {
                let mut start: Option<SystemTime> = None;
                loop {
                    if let Ok(runtime) = &mut runtime.try_write() {
                        if let Some($field_name_ident) = runtime.deref_mut().$field_name_ident.as_mut() {
                            if let Some(start) = start {
                                debug!("Runtime: `{}` became available after a {:?} wait", $field_name_str, start.elapsed().unwrap());
                            }
                            break callback($field_name_ident).await
                        }
                    }
                    if let Some(_start) = start {
                        if _start.elapsed().unwrap() > TIMEOUT {
                            panic!("Could not retrieve `{}` instance: {}",
                                   $field_name_str,
                                   if let Ok(_runtime) = &runtime.try_read() {
                                       format!("it was not registered in `Runtime` even after {:?}", TIMEOUT)
                                   } else {
                                       format!("`Runtime` seems to be locked elsewhere for the past {:?}", TIMEOUT)
                                });
                        }
                    } else {
                        start = Some(SystemTime::now());
                        debug!("Runtime: `{}` is not (yet?) available. Waiting for up to {:?} for main.rs to finish instantiating it and placing it here with `register_{}()`",
                               $field_name_str, TIMEOUT, $field_name_str);
                    }
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            }

            /// Similar to [$get_function_name()], but only executes callback if the value is present -- not failing if it is not.
            pub async fn $opt_get_function_name<ReturnType>
                                               (runtime:  &RwLock<Self>,
                                                callback: impl for<'r> FnOnce(&'r mut $field_type) -> BoxFuture<'r, ReturnType> + Send)
                                               -> Option<ReturnType> {
                {
                    let locked_runtime = &runtime.write().await;
                    if let None = &locked_runtime.$field_name_ident {
                        return None
                    }
                }
                Some(Self::$get_function_name(runtime, callback).await)
            }

        }
    }
}

impl Runtime {

    pub fn new(executable_path: String) -> Self {
        Self {
            executable_path,
            tokio_runtime: None,
            ogre_robot:    None,
            socket_server: None,
        }
    }
}

// implements getters and setters for all `Option` fields that are to be set/get asynchronously
///////////////////////////////////////////////////////////////////////////////////////////////
impl_runtime!("ogre_robot",    ogre_robot,    OgreRobot,               register_ogre_robot,    do_for_ogre_robot,    do_if_ogre_robot_is_present);
impl_runtime!("socket_server", socket_server, SocketServer<'static>,   register_socket_server, do_for_socket_server, do_if_socket_server_is_present);
