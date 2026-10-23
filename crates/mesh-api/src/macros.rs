//! `string_enum!`: declares a closed set of JSON string values (a TS string-literal union).
//!
//! Every generated enum gets serde (exact wire strings), `ts_rs::TS` under the
//! `ts-export` feature, an `ALL` list in declaration order, `as_str()` and
//! `FromStr`/`Display`. Because `ALL` and `as_str()` are generated from the same variant
//! list, they can never drift from the type, which is what `constants.ts` and the TS
//! guards rely on (DESIGN §3.7).

/// Declares a string enum: `string_enum! { /// docs  pub enum Name { /// docs  Variant = "wire", ... } }`.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $wire:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
            ::serde::Serialize, ::serde::Deserialize,
        )]
        #[cfg_attr(feature = "ts-export", derive(::ts_rs::TS), ts(export))]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $wire)] $variant, )+
        }

        impl $name {
            /// Every value, in declaration order (exported to TS by `cargo xtask gen-ts`).
            pub const ALL: &'static [$name] = &[ $( $name::$variant ),+ ];

            /// The exact JSON string of this value.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( $name::$variant => $wire, )+
                }
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::core::str::FromStr for $name {
            type Err = $crate::UnknownValue;

            fn from_str(s: &str) -> ::core::result::Result<Self, Self::Err> {
                match s {
                    $( $wire => Ok($name::$variant), )+
                    _ => Err($crate::UnknownValue {
                        type_name: stringify!($name),
                        value: s.to_owned(),
                    }),
                }
            }
        }
    };
}

pub(crate) use string_enum;
