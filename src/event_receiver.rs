use std::ops::Deref;
use std::sync::OnceLock;

use jni::{objects::JString, refs::Reference, Env};
use jni_min_helper::{BroadcastReceiver, Intent, IntentFilter};
use log::{error, info};

use crate::async_util::{Notifier, NotifierReceiver};
use crate::bindings;
use crate::gatt_tree::GattTree;
use crate::util::{jni_with_env, ReferenceExt};
use crate::DeviceId;

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, PartialEq)]
pub enum GlobalEvent {
    /// contains EXTRA_STATE
    AdapterStateChanged(i32),
    /// `Adapter::scan` should return when this event is received
    DiscoveryFinished,
    /// contains device address
    #[allow(unused)] // NOTE: this may not be received; this can be removed.
    AclConnectionStateChanged(DeviceId, bool),
    /// contains device address, EXTRA_PREVIOUS_BOND_STATE, and EXTRA_BOND_STATE
    BondStateChanged(DeviceId, i32, i32),
}

static GLOBAL_STORE: OnceLock<EventReceiverInner> = OnceLock::new();

static RELEVANT_ACTIONS: &[&str] = &[
    bindings::BluetoothAdapter::ACTION_STATE_CHANGED,
    bindings::BluetoothAdapter::ACTION_DISCOVERY_FINISHED,
    bindings::BluetoothDevice::ACTION_ACL_CONNECTED,
    bindings::BluetoothDevice::ACTION_ACL_DISCONNECTED,
    bindings::BluetoothDevice::ACTION_BOND_STATE_CHANGED,
];

pub struct EventReceiver;

struct EventReceiverInner {
    notifier: Notifier<GlobalEvent>,
    java_receiver: BroadcastReceiver,
}

impl EventReceiverInner {
    fn get() -> Result<&'static Self, crate::Error> {
        if GLOBAL_STORE.get().is_none() {
            let notifier = Notifier::new(128);
            let java_receiver = BroadcastReceiver::build(move |env, _, intent| {
                let notifier = &GLOBAL_STORE.get().unwrap().notifier;
                on_receive(notifier, env, intent)
            })?;
            let _ = GLOBAL_STORE.set(Self {
                notifier,
                java_receiver,
            });
        }
        Ok(GLOBAL_STORE.get().unwrap())
    }
}

impl EventReceiver {
    pub async fn subscribe() -> Result<NotifierReceiver<GlobalEvent>, crate::Error> {
        EventReceiverInner::get()?
            .notifier
            .subscribe(
                async move {
                    jni_with_env(|env| {
                        let filter = IntentFilter::new(env)?;
                        for action in RELEVANT_ACTIONS {
                            let action = JString::new(env, action)?;
                            filter.add_action(env, &action)?;
                        }
                        let inner = EventReceiverInner::get()?;
                        info!("registering the global bluetooth event broadcast receiver.");
                        inner.java_receiver.register(&filter)?;
                        Ok(())
                    })
                },
                || {
                    let _ = jni_with_env(|_| {
                        let inner = EventReceiverInner::get()?;
                        info!("deregistering the global bluetooth event broadcast receiver.");
                        inner.java_receiver.unregister()?;
                        Ok(())
                    });
                },
            )
            .await
    }
}

fn on_receive<'local>(
    notifier: &Notifier<GlobalEvent>,
    env: &mut Env<'local>,
    intent: Intent<'local>,
) -> Result<(), jni::errors::Error> {
    let Some(intent) = intent.to_option() else {
        return Ok(());
    };
    let mut get_action =
        |intent: &Intent<'_>| Ok::<_, crate::Error>(intent.get_action(env)?.to_string());
    let Ok(action) = get_action(&intent) else {
        error!("failed to get the action string of the received intent");
        return Ok(());
    };
    let mut process_intent = || match action.trim() {
        bindings::BluetoothAdapter::ACTION_STATE_CHANGED => {
            let extra_state = bindings::BluetoothAdapter::EXTRA_STATE(env)?;
            let val = intent.get_int_extra(env, &extra_state, 0)?;
            if val == bindings::BluetoothAdapter::STATE_OFF {
                // XXX: or STATE_TURNING_OFF?
                if GattTree::clear_connections() {
                    info!("deregistered all connections in BroadcastReceiverProxy");
                }
            }
            notifier.notify(GlobalEvent::AdapterStateChanged(val));
            Ok::<(), crate::Error>(())
        }
        bindings::BluetoothAdapter::ACTION_DISCOVERY_FINISHED => {
            notifier.notify(GlobalEvent::DiscoveryFinished);
            Ok(())
        }
        bindings::BluetoothDevice::ACTION_ACL_CONNECTED => {
            let extra_transport = bindings::BluetoothDevice::EXTRA_TRANSPORT(env)?;
            let transport = intent.get_int_extra(env, &extra_transport, 0)?;
            if transport == bindings::BluetoothDevice::TRANSPORT_LE {
                let dev_id = get_extra_device_id(env, &intent)?;
                notifier.notify(GlobalEvent::AclConnectionStateChanged(dev_id, true));
            }
            Ok(())
        }
        bindings::BluetoothDevice::ACTION_ACL_DISCONNECTED => {
            let extra_transport = bindings::BluetoothDevice::EXTRA_TRANSPORT(env)?;
            let transport = intent.get_int_extra(env, &extra_transport, 0)?;
            if transport == bindings::BluetoothDevice::TRANSPORT_LE {
                let dev_id = get_extra_device_id(env, &intent)?;
                if GattTree::deregister_connection(&dev_id) {
                    info!("deregistered connection with {dev_id} in BroadcastReceiverProxy");
                }
                notifier.notify(GlobalEvent::AclConnectionStateChanged(dev_id, false));
            }
            Ok(())
        }
        bindings::BluetoothDevice::ACTION_BOND_STATE_CHANGED => {
            let dev_id = get_extra_device_id(env, &intent)?;
            let extra_prev_bond_state = bindings::BluetoothDevice::EXTRA_PREVIOUS_BOND_STATE(env)?;
            let prev_bond_state = intent.get_int_extra(env, &extra_prev_bond_state, 0)?;
            let extra_bond_state = bindings::BluetoothDevice::EXTRA_BOND_STATE(env)?;
            let bond_state = intent.get_int_extra(env, &extra_bond_state, 0)?;
            notifier.notify(GlobalEvent::BondStateChanged(
                dev_id,
                prev_bond_state,
                bond_state,
            ));
            Ok(())
        }
        _ => Ok(()),
    };
    if let Err(e) = process_intent() {
        error!("failed to get the extra value of the received intent: {e}");
    }
    Ok(())
}

fn get_extra_device_id<'local>(
    env: &mut Env<'local>,
    intent: &Intent<'local>,
) -> Result<DeviceId, crate::Error> {
    let extra_device = bindings::BluetoothDevice::EXTRA_DEVICE(env)?;
    let device_class =
        bindings::BluetoothDevice::lookup_class(env, &jni::objects::LoaderContext::None)?;
    let device = intent.get_parcelable_extra(env, &extra_device, device_class.deref())?;
    if device.is_null() {
        return Err(crate::Error::new(
            crate::error::ErrorKind::Internal,
            None,
            "failed to get EXTRA_DEVICE from received intent",
        ));
    }
    let device = env.as_cast::<bindings::BluetoothDevice>(&device)?;
    Ok(DeviceId::from_java_dev(env, device)?)
}
