use crate::alloc::borrow::Cow;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, Error, Result};

use crate::common::{member_as_idents, struct_fields};

pub fn derive(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;
    let data = match input.data {
        syn::Data::Struct(s) => s,
        _ => {
            return Err(Error::new_spanned(
                ident,
                "derive(Bundle) does not support enums or unions",
            ))
        }
    };
    let (tys, field_members) = struct_fields(&data.fields);
    let field_idents = member_as_idents(&field_members);
    let generics = add_additional_bounds_to_generic_params(input.generics);

    let dyn_bundle_code = gen_dynamic_bundle_impl(&ident, &generics, &field_members, &tys);
    let bundle_code = if tys.is_empty() {
        gen_unit_struct_bundle_impl(ident, &generics)
    } else {
        gen_bundle_impl(&ident, &generics, &field_members, &field_idents, &tys)
    };
    let mut ts = dyn_bundle_code;
    ts.extend(bundle_code);
    Ok(ts)
}

fn gen_dynamic_bundle_impl(
    ident: &syn::Ident,
    generics: &syn::Generics,
    field_members: &[syn::Member],
    tys: &[&syn::Type],
) -> TokenStream2 {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        unsafe impl #impl_generics ::hecs::DynamicBundle for #ident #ty_generics #where_clause {
            fn key(&self) -> ::core::option::Option<::core::any::TypeId> {
                ::core::option::Option::Some(::core::any::TypeId::of::<Self>())
            }

            fn with_ids<__hecs__T>(&self, f: impl ::core::ops::FnOnce(&[::core::any::TypeId]) -> __hecs__T) -> __hecs__T {
                <Self as ::hecs::Bundle>::with_static_ids(f)
            }

            fn type_info(&self) -> ::hecs::alloc::vec::Vec<::hecs::TypeInfo> {
                <Self as ::hecs::Bundle>::with_static_type_info(|info| info.to_vec())
            }

            #[allow(clippy::forget_copy, clippy::forget_non_drop)]
            unsafe fn put(mut self, mut f: impl ::core::ops::FnMut(*mut u8, ::hecs::TypeInfo)) {
                #(
                    f((&mut self.#field_members as *mut #tys).cast::<u8>(), ::hecs::TypeInfo::of::<#tys>());
                    ::core::mem::forget(self.#field_members);
                )*
            }
        }
    }
}

fn gen_bundle_impl(
    ident: &syn::Ident,
    generics: &syn::Generics,
    field_members: &[syn::Member],
    field_idents: &[Cow<syn::Ident>],
    tys: &[&syn::Type],
) -> TokenStream2 {
    let num_tys = tys.len();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let with_static_ids_inner = quote! {
        {
            let mut tys = [#((::core::mem::align_of::<#tys>(), ::core::any::TypeId::of::<#tys>())),*];
            tys.sort_unstable_by(|x, y| {
                ::core::cmp::Ord::cmp(&x.0, &y.0)
                    .reverse()
                    .then(::core::cmp::Ord::cmp(&x.1, &y.1))
            });
            let mut ids = [::core::any::TypeId::of::<()>(); #num_tys];
            for (id, info) in ::core::iter::Iterator::zip(ids.iter_mut(), tys.iter()) {
                *id = info.1;
            }
            ids
        }
    };
    let with_static_ids_body = if generics.params.is_empty() {
        quote! {
            static ELEMENTS: ::hecs::once_cell::race::OnceBox<[::core::any::TypeId; #num_tys]> = ::hecs::once_cell::race::OnceBox::new();
            f(ELEMENTS.get_or_init(||
                ::hecs::alloc::boxed::Box::new(#with_static_ids_inner)
            ))
        }
    } else {
        quote! {
            f(&#with_static_ids_inner)
        }
    };
    quote! {
        unsafe impl #impl_generics ::hecs::Bundle for #ident #ty_generics #where_clause {
            #[allow(non_camel_case_types)]
            fn with_static_ids<__hecs__T>(f: impl ::core::ops::FnOnce(&[::core::any::TypeId]) -> __hecs__T) -> __hecs__T {
                #with_static_ids_body
            }

            #[allow(non_camel_case_types)]
            fn with_static_type_info<__hecs__T>(f: impl ::core::ops::FnOnce(&[::hecs::TypeInfo]) -> __hecs__T) -> __hecs__T {
                let mut info: [::hecs::TypeInfo; #num_tys] = [#(::hecs::TypeInfo::of::<#tys>()),*];
                info.sort_unstable();
                f(&info)
            }

            unsafe fn get(
                mut f: impl ::core::ops::FnMut(::hecs::TypeInfo) -> ::core::option::Option<::core::ptr::NonNull<u8>>,
            ) -> ::core::result::Result<Self, ::hecs::MissingComponent> {
                #(
                    let #field_idents = f(::hecs::TypeInfo::of::<#tys>())
                            .ok_or_else(::hecs::MissingComponent::new::<#tys>)?
                            .cast::<#tys>()
                            .as_ptr();
                )*
                ::core::result::Result::Ok(Self { #( #field_members: #field_idents.read(), )* })
            }
        }
    }
}

// no reason to generate a static for unit structs
fn gen_unit_struct_bundle_impl(ident: syn::Ident, generics: &syn::Generics) -> TokenStream2 {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        unsafe impl #impl_generics ::hecs::Bundle for #ident #ty_generics #where_clause {
            #[allow(non_camel_case_types)]
            fn with_static_ids<__hecs__T>(f: impl ::core::ops::FnOnce(&[::core::any::TypeId]) -> __hecs__T) -> __hecs__T { f(&[]) }
            #[allow(non_camel_case_types)]
            fn with_static_type_info<__hecs__T>(f: impl ::core::ops::FnOnce(&[::hecs::TypeInfo]) -> __hecs__T) -> __hecs__T { f(&[]) }

            unsafe fn get(
                mut f: impl ::core::ops::FnMut(::hecs::TypeInfo) -> ::core::option::Option<::core::ptr::NonNull<u8>>,
            ) -> ::core::result::Result<Self, ::hecs::MissingComponent> {
                ::core::result::Result::Ok(Self {/* for some reason this works for all unit struct variations */})
            }
        }
    }
}

fn make_component_trait_bound() -> syn::TraitBound {
    syn::TraitBound {
        paren_token: None,
        modifier: syn::TraitBoundModifier::None,
        lifetimes: None,
        path: syn::parse_quote!(::hecs::Component),
    }
}

fn add_additional_bounds_to_generic_params(mut generics: syn::Generics) -> syn::Generics {
    generics.type_params_mut().for_each(|tp| {
        tp.bounds
            .push(syn::TypeParamBound::Trait(make_component_trait_bound()))
    });
    generics
}
