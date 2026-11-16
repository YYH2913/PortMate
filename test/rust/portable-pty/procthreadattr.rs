use super::*;

#[test]
fn native_attribute_storage_is_initialized_and_pointer_aligned() {
    let mut list = ProcThreadAttributeList::with_capacity(1).unwrap();
    assert!(!list.data.is_empty());
    assert_eq!(list.as_mut_ptr() as usize % mem::align_of::<usize>(), 0);
}
