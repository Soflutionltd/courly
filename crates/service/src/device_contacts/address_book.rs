//! Reads and writes the Mac's address book through the Contacts framework.
//! What Courly writes there reaches the iPhone through iCloud.

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
    CNContactOrganizationNameKey, CNContactPhoneNumbersKey, CNContactStore, CNEntityType,
    CNKeyDescriptor, CNLabelOther, CNLabelPhoneNumberMobile, CNLabeledValue, CNMutableContact,
    CNPhoneNumber, CNSaveRequest,
};
use objc2_foundation::{NSArray, NSError, NSMutableCopying, NSString};

use super::DeviceContact;
use crate::actions::contact_fields::{ContactFields, ValueEdit, same_address, same_phone};

pub(crate) enum StoreError {
    /// The user refused access, or a policy forbids it.
    Denied,
    Failed(String),
}

/// Every person in the address book, asking the user for access the
/// first time. Blocks until the user answers that prompt.
pub(crate) fn read() -> Result<Vec<DeviceContact>, StoreError> {
    let store = authorized_store()?;
    let keys = fetched_keys();
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
        return Err(StoreError::Failed(error.map_or_else(
            || "the address book enumeration stopped without an error".to_owned(),
            |error| error.localizedDescription().to_string(),
        )));
    }
    drop(collect);
    Ok(contacts.into_inner())
}

/// Adds a person to the default address book (iCloud when the Mac syncs
/// its contacts with iCloud). Returns the new person's identifier.
pub(crate) fn create(fields: &ContactFields) -> Result<String, StoreError> {
    let store = authorized_store()?;
    let contact = unsafe { CNMutableContact::new() };
    let (given, family) = fields.split_name();
    let emails: Vec<_> = fields
        .emails()
        .iter()
        .map(|email| email_value(email))
        .collect();
    let phones = match fields.phone.as_deref() {
        Some(phone) => vec![phone_value(phone)?],
        None => Vec::new(),
    };
    unsafe {
        contact.setGivenName(&NSString::from_str(&given));
        contact.setFamilyName(&NSString::from_str(&family));
        contact.setOrganizationName(&NSString::from_str(fields.company.as_deref().unwrap_or("")));
        contact.setEmailAddresses(&NSArray::from_retained_slice(&emails));
        contact.setPhoneNumbers(&NSArray::from_retained_slice(&phones));
    }
    let request = unsafe { CNSaveRequest::new() };
    unsafe { request.addContact_toContainerWithIdentifier(&contact, None) };
    save(&store, &request)?;
    Ok(unsafe { contact.identifier() }.to_string())
}

/// Applies an edit to a person, leaving the values Courly does not show
/// (other numbers, other addresses, labels) as they are.
pub(crate) fn update(
    identifier: &str,
    old: &ContactFields,
    new: &ContactFields,
) -> Result<(), StoreError> {
    let store = authorized_store()?;
    let contact = fetch(&store, identifier)?.mutableCopy();
    if old.display_name != new.display_name {
        let (given, family) = new.split_name();
        unsafe {
            contact.setGivenName(&NSString::from_str(&given));
            contact.setFamilyName(&NSString::from_str(&family));
        }
    }
    if old.company != new.company {
        unsafe {
            contact.setOrganizationName(&NSString::from_str(new.company.as_deref().unwrap_or("")));
        }
    }

    let mut emails = unsafe { contact.emailAddresses() }.to_vec();
    for edit in [
        ValueEdit::between(Some(&old.email), Some(&new.email)),
        ValueEdit::between(old.email2.as_deref(), new.email2.as_deref()),
    ] {
        edit.apply(
            &mut emails,
            |item| unsafe { item.value() }.to_string(),
            |item, value| {
                *item = unsafe { item.labeledValueBySettingValue(&NSString::from_str(value)) };
            },
            email_value,
            same_address,
        );
    }
    unsafe { contact.setEmailAddresses(&NSArray::from_retained_slice(&emails)) };

    let phone_edit = ValueEdit::between(old.phone.as_deref(), new.phone.as_deref());
    if phone_edit != ValueEdit::Keep {
        let mut phones = unsafe { contact.phoneNumbers() }.to_vec();
        let text = |item: &Retained<CNLabeledValue<CNPhoneNumber>>| {
            unsafe { item.value().stringValue() }.to_string()
        };
        match phone_edit {
            ValueEdit::Keep => {}
            ValueEdit::Remove(old) => phones.retain(|item| !same_phone(&text(item), old)),
            ValueEdit::Replace { new, .. } | ValueEdit::Add(new) => {
                let number = phone_number(new)?;
                let replaced = match phone_edit {
                    ValueEdit::Replace { old, .. } => {
                        phones.iter().position(|item| same_phone(&text(item), old))
                    }
                    _ => None,
                };
                match replaced {
                    Some(index) => {
                        phones[index] =
                            unsafe { phones[index].labeledValueBySettingValue(&number) };
                    }
                    None if phones.iter().any(|item| same_phone(&text(item), new)) => {}
                    None => phones.push(unsafe {
                        CNLabeledValue::labeledValueWithLabel_value(
                            Some(CNLabelPhoneNumberMobile),
                            &number,
                        )
                    }),
                }
            }
        }
        unsafe { contact.setPhoneNumbers(&NSArray::from_retained_slice(&phones)) };
    }

    let request = unsafe { CNSaveRequest::new() };
    unsafe { request.updateContact(&contact) };
    save(&store, &request)
}

/// Removes a person from the address book.
pub(crate) fn delete(identifier: &str) -> Result<(), StoreError> {
    let store = authorized_store()?;
    let contact = fetch(&store, identifier)?.mutableCopy();
    let request = unsafe { CNSaveRequest::new() };
    unsafe { request.deleteContact(&contact) };
    save(&store, &request)
}

/// The store, once the user granted access; asks the first time.
fn authorized_store() -> Result<Retained<CNContactStore>, StoreError> {
    // SAFETY: CNContactStore is documented thread-safe, and every call in
    // this module passes the framework the argument types its headers
    // declare.
    let store = unsafe { CNContactStore::new() };
    let status =
        unsafe { CNContactStore::authorizationStatusForEntityType(CNEntityType::Contacts) };
    if status == CNAuthorizationStatus::NotDetermined {
        request_access(&store)?;
    } else if status != CNAuthorizationStatus::Authorized
        && status != CNAuthorizationStatus::Limited
    {
        return Err(StoreError::Denied);
    }
    Ok(store)
}

fn fetched_keys() -> Retained<NSArray<ProtocolObject<dyn CNKeyDescriptor>>> {
    unsafe {
        NSArray::from_slice(&[
            ProtocolObject::from_ref(CNContactIdentifierKey),
            ProtocolObject::from_ref(CNContactGivenNameKey),
            ProtocolObject::from_ref(CNContactFamilyNameKey),
            ProtocolObject::from_ref(CNContactOrganizationNameKey),
            ProtocolObject::from_ref(CNContactEmailAddressesKey),
            ProtocolObject::from_ref(CNContactPhoneNumbersKey),
        ])
    }
}

fn fetch(store: &CNContactStore, identifier: &str) -> Result<Retained<CNContact>, StoreError> {
    unsafe {
        store.unifiedContactWithIdentifier_keysToFetch_error(
            &NSString::from_str(identifier),
            &fetched_keys(),
        )
    }
    .map_err(|error| StoreError::Failed(error.localizedDescription().to_string()))
}

fn save(store: &CNContactStore, request: &CNSaveRequest) -> Result<(), StoreError> {
    unsafe { store.executeSaveRequest_error(request) }
        .map_err(|error| StoreError::Failed(error.localizedDescription().to_string()))
}

fn email_value(email: &str) -> Retained<CNLabeledValue<NSString>> {
    unsafe {
        CNLabeledValue::labeledValueWithLabel_value(Some(CNLabelOther), &NSString::from_str(email))
    }
}

fn phone_number(phone: &str) -> Result<Retained<CNPhoneNumber>, StoreError> {
    unsafe { CNPhoneNumber::phoneNumberWithStringValue(&NSString::from_str(phone)) }
        .ok_or_else(|| StoreError::Failed(format!("\"{phone}\" is not a phone number")))
}

fn phone_value(phone: &str) -> Result<Retained<CNLabeledValue<CNPhoneNumber>>, StoreError> {
    let number = phone_number(phone)?;
    Ok(unsafe {
        CNLabeledValue::labeledValueWithLabel_value(Some(CNLabelPhoneNumberMobile), &number)
    })
}

fn request_access(store: &CNContactStore) -> Result<(), StoreError> {
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
        Ok(false) => Err(StoreError::Denied),
        Err(error) => Err(StoreError::Failed(format!(
            "the access request ended without an answer: {error}"
        ))),
    }
}

/// # Safety
///
/// `contact` must have been fetched with the keys of `fetched_keys`.
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
            phones: contact
                .phoneNumbers()
                .iter()
                .map(|labeled| labeled.value().stringValue().to_string())
                .collect(),
        }
    }
}
