" Vim syntax file
" Language: TriCTI
" Maintainer: TriCTI Developers
" Latest Revision: 2025-10-02

if exists("b:current_syntax")
  finish
endif

syntax case match

" Comments
syntax match trictiComment "#.*$"
syntax region trictiComment start="/\*" end="\*/" contains=trictiComment keepend
highlight def link trictiComment Comment

" Attributes (decorators)
syntax match trictiAttribute "@[A-Za-z_][A-Za-z0-9_]*"
highlight def link trictiAttribute PreProc

" Keywords
syntax keyword trictiKeywordControl if else while for match ret continue break in async await extern loop then
syntax keyword trictiKeywordOther db table compose enum impl trait use mod pub type where self Self super
syntax keyword trictiKeywordSystem sys new emit
syntax keyword trictiKeywordQuery select from where on join inner left right full query res
highlight def link trictiKeywordControl Conditional
highlight def link trictiKeywordOther Keyword
highlight def link trictiKeywordSystem Keyword
highlight def link trictiKeywordQuery Keyword

" Storage types and literals
syntax keyword trictiType i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 bool char str string none some ok
syntax keyword trictiBoolean true false
highlight def link trictiType Type
highlight def link trictiBoolean Boolean

" Strings
syntax region trictiString start='"' skip='\\.' end='"'
syntax region trictiString start="'" skip='\\.' end="'" contains=trictiEscape
syntax match trictiEscape "\\."
highlight def link trictiString String
highlight def link trictiEscape SpecialChar

" Numbers
syntax match trictiNumber "\<0x[0-9A-Fa-f]\+\>"
syntax match trictiNumber "\<0b[01]\+\>"
syntax match trictiNumber "\<0o[0-7]\+\>"
syntax match trictiNumber "\<[0-9]\+\(\.[0-9]\+\)\?\(f32\|f64\)?\>"
syntax match trictiNumber "\<[0-9]\+\(i8\|i16\|i32\|i64\|i128\|u8\|u16\|u32\|u64\|u128\)?\>"
highlight def link trictiNumber Number

" Operators
syntax match trictiOperator /\V::/
syntax match trictiOperator /\V->/
syntax match trictiOperator /\V?->/
syntax match trictiOperator /\V=>/
syntax match trictiOperator /\V:=/
syntax match trictiOperator /\V==/
syntax match trictiOperator /\V!=/
syntax match trictiOperator /\V<=/
syntax match trictiOperator /\V>=/
syntax match trictiOperator /\V~=/
syntax match trictiOperator /\V<</
syntax match trictiOperator /\V>>/
syntax match trictiOperator /\V..=/
syntax match trictiOperator /\V../
syntax match trictiOperator /\V&&/
syntax match trictiOperator /\V||/
syntax match trictiOperator /[+=\-*/%!~&|^<>]/
highlight def link trictiOperator Operator

" Function definitions and calls
syntax match trictiFunction "\<[A-Za-z_][A-Za-z0-9_]*\>\ze\s*::"
syntax match trictiFunctionCall "\<[a-zA-Z_][A-Za-z0-9_]*\>\ze\s*(" contains=trictiKeywordControl,trictiKeywordOther,trictiType,trictiBoolean nextgroup=trictiCallArgs skipwhite
highlight def link trictiFunction Function
highlight def link trictiFunctionCall Identifier

" Types starting with uppercase
syntax match trictiUserType "\<[A-Z][A-Za-z0-9_]*\>"
highlight def link trictiUserType Type

let b:current_syntax = "tricti"
