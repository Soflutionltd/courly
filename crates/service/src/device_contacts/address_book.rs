//! Reads the Mac's address book through the Contacts framework.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::mpsc;

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2_contacts::{
    CNAuthorizationStatus, CNContact, CNContactEmailAddressesKey, CNContactFamilyNameKey,
    CNContactFetchRequest, CNContactGivenNameKey, CNContactIdentifierKey,
    CNContactOrganizationNameKey, CNContactStore, CNEntityType, CNKeyDescriptor,
};
use objc2_foundation::{NSArray, NSError};

use super::DeviceContact;

pub(crate) enum ReadError {
    /// The user refused access, or a policy forbids it.
    Denied,
    Failed(String),
}

/// Every person in the address book, asking the user for access the
/// first time. Blocks until the user answers that prompt.
pub(crate) fn read() -> Result<Vec<DeviceContact>, ReadError> {
    // SAFETY: CNContactStore is documented thread-safe, and every call below
    // passes the framework the argument types its headers declare.
    let store = unsafe { CNContactStore::new() };
    let status =
        unsafe { CNContactStore::authorizationStatusForEntityType(CNEntityType::Contacts) };
    if status == CNAuthorizationStatus::NotDetermined {
        request_access(&store)?;
    } else if status != CNAuthorizationStatus::Authorized
        && status != CNAuthorizationStatus::Limited
    {
        return Err(ReadError::Denied);
    }

    let keys: Retained<NSArray<ProtocolObject<dyn CNKeyDescriptor>>> = unsafe {
        NSArray::from_slice(&[
            ProtocolObject::from_ref(CNContactIdentifierKey),
            ProtocolObject::from_ref(CNContactGivenNameKey),
            ProtocolObject::from_ref(CNContactFamilyNameKey),
            ProtocolObject::from_ref(CNContactOrganizationNameKey),
            ProtocolObject::from_ref(CNContactEmailAddressesKey),
        ])
    };
    let request = unsafe {
        CNContactFetchRequest::initWithKeysToFetch(CNContactFetchRequest::alloc(), &keys)
    };

    let contacts = RefCell::new(Vec::new());
    let collect = RcBlock::new(|contact: NonNull<CNContact>, _stop: NonNull<Bool>| {
        // SAFETY: the framework hands the block a valid contact for the
        // duration of the call, fetched with the keys read here.
        let contact = unsafe { contact.as_ref() };
        contacts
            .borrow_mut()
            .push(unsafe { device_contact(contact) });
    });
    let mut error: Option<Retained<NSError>> = None;
    let completed = unsafe {
        store.enumerateContactsWithFetchRequest_error_usingBlock(
            &request,
            Some(&mut error),
            &collect,
        )
    };
    if !completed {
        return Err(ReadError::Failed(error.map_or_else(
            || "the address book enumeration stopped without an error".to_owned(),
            |error| error.localizedDescription().to_string(),
        )));
    }
    drop(collect);
    Ok(contacts.into_inner())
}

fn request_access(store: &CNContactStore) -> Result<(), ReadError> {
    let (answer_tx, answer_rx) = mpsc::channel();
    let on_answer = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        if let Err(error) = answer_tx.send(granted.as_bool()) {
            log::warn!("device contacts: access answer arrived after the request ended: {error}");
        }
    });
    unsafe {
        store.requestAccessForEntityType_completionHandler(CNEntityType::Contacts, &on_answer);
    }
    match answer_rx.recv() {
        Ok(true) => Ok(()),
        Ok(false) => Err(ReadError::Denied),
        Err(error) => Err(ReadError::Failed(format!(
            "the access request ended without an answer: {error}"
        ))),
    }
}

/// # Safety
///
/// `contact` must have been fetched with the identifier, name,
/// organization and email keys.
unsafe fn device_contact(contact: &CNContact) -> DeviceContact {
    unsafe {
        DeviceContact {
            identifier: contact.identifier().to_string(),
            given_name: contact.givenName().to_string(),
            family_name: contact.familyName().to_string(),
            organization: contact.organizationName().to_string(),
            emails: contact
                .emailAddresses()
                .iter()
                .map(|labeled| labeled.value().to_string())
                .collect(),
        }
    }
}
