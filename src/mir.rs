use std::collections::HashMap;
use internment::LocalIntern;
use crate::{
    ast::{self, Literal},
    ty::{Type, Primitive},
    error::Error,
    node::RawTypeNode,
    hir::self,
};

type Ident = LocalIntern<String>;

pub type Unary = ast::UnaryOp;
pub type Binary = ast::BinaryOp;

pub type Intrinsic = hir::Intrinsic;

// Raw types have had their name erased (since type checking and inference has already occurred)
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum RawType {
    Primitive(Primitive),
    List(Box<Self>),
    Product(Vec<Self>),
    Sum(Vec<Self>),
    Func(Box<Self>, Box<Self>),
    Boxed(Ident, Vec<Self>),
}

impl RawType {
    pub fn mangle(&self) -> String {
        match self {
            RawType::Primitive(prim) => format!("{}", prim),
            RawType::List(item) => format!("[{}]", item.mangle()),
            RawType::Product(items) => format!("({})", items
                .iter()
                .map(|item| item.mangle())
                .collect::<Vec<_>>()
                .join(", "),
            ),
            RawType::Sum(items) => format!("({})", items
                .iter()
                .map(|item| item.mangle())
                .collect::<Vec<_>>()
                .join(" | "),
            ),
            RawType::Func(i, o) => format!("{} -> {}", i.mangle(), o.mangle()),
            RawType::Boxed(name, params) => format!(
                "{}{}{}",
                name,
                if params.len() > 0 { " " } else { "" },
                params
                    .iter()
                    .map(|param| format!(" {}", param.mangle()))
                    .collect::<Vec<_>>()
                    .join(""),
            ),
        }
    }
}

impl hir::TypeBinding {
    fn make_matcher(&self, prog: &hir::Program) -> Matcher {
        match &*self.pat {
            hir::Pat::Wildcard => Matcher::Wildcard,
            hir::Pat::Literal(litr) => Matcher::Exactly(litr.clone()),
            hir::Pat::Tuple(items) => Matcher::Product(items
                .iter()
                .map(|item| item.make_matcher(prog))
                .collect()),
            hir::Pat::Record(fields) => Matcher::Product(fields
                .iter()
                .map(|(_, field)| field.make_matcher(prog))
                .collect()),
            hir::Pat::List(items) => Matcher::List(items
                .iter()
                .map(|item| item.make_matcher(prog))
                .collect()),
            hir::Pat::ListFront(items, _) => Matcher::ListFront(items
                .iter()
                .map(|item| item.make_matcher(prog))
                .collect()),
            hir::Pat::Deconstruct(data, _, inner) => {
                if prog.data_ctx.get_data(data.0).variants.len() == 1 {
                    inner.make_matcher(prog)
                } else {
                    Matcher::Product(vec![
                        Matcher::Exactly(Literal::Number(data.1 as f64)),
                        inner.make_matcher(prog),
                    ])
                }
            },
        }
    }

    fn make_extractor(&self, prog: &hir::Program) -> Extractor {
        match &*self.pat {
            hir::Pat::Wildcard | hir::Pat::Literal(_) =>
                Extractor::Just(self.binding.as_ref().map(|ident| **ident)),
            hir::Pat::Tuple(items) => Extractor::Product(
                self.binding.as_ref().map(|ident| **ident),
                items.iter().map(|item| item.make_extractor(prog)).collect(),
            ),
            hir::Pat::Record(fields) => Extractor::Product(
                self.binding.as_ref().map(|ident| **ident),
                fields.iter().map(|(_, field)| field.make_extractor(prog)).collect(),
            ),
            hir::Pat::List(items) => Extractor::List(
                self.binding.as_ref().map(|ident| **ident),
                items.iter().map(|item| item.make_extractor(prog)).collect(),
            ),
            hir::Pat::ListFront(items, tail) => Extractor::ListFront(
                self.binding.as_ref().map(|ident| **ident),
                items.iter().map(|item| item.make_extractor(prog)).collect(),
                tail.as_ref().map(|ident| **ident),
            ),
            hir::Pat::Deconstruct(data, _, inner) => {
                if prog.data_ctx.get_data(data.0).variants.len() == 1 {
                    inner.make_extractor(prog)
                } else {
                    Extractor::Product(
                        None,
                        vec![
                            Extractor::Just(None),
                            inner.make_extractor(prog),
                        ],
                    )
                }
            },
        }
    }
}

pub type DefId = LocalIntern<(Ident, Vec<RawType>)>;

#[derive(Debug)]
pub enum Expr {
    Literal(Literal),
    // Get the value of the given global
    GetGlobal(DefId),
    // Get the value of the given local
    GetLocal(Ident),
    // Evaluate an intrinsic operation
    Intrinsic(Intrinsic, Vec<RawTypeNode<Self>>),
    // Perform a built-in unary operation
    Unary(Unary, RawTypeNode<Self>),
    // Perform a built-in binary operation
    Binary(Binary, RawTypeNode<Self>, RawTypeNode<Self>),
    // Construct a tuple with the given values
    Tuple(Vec<RawTypeNode<Self>>),
    // Construct a list with the given values
    List(Vec<RawTypeNode<Self>>),
    // Apply a value to a function
    Apply(RawTypeNode<Self>, RawTypeNode<Self>),
    // Access the field of a Tuple
    Access(RawTypeNode<Self>, usize),
    // Update the field of a Tuple
    Update(RawTypeNode<Self>, usize, Ident, RawTypeNode<Self>),
    // Create a function with the given parameter extractor and body
    Func(Extractor, Vec<Ident>, RawTypeNode<Self>),
    // Make a flat value against a series of arms
    Match(RawTypeNode<Self>, Vec<(Matcher, Extractor, RawTypeNode<Self>)>),
}

impl hir::TypeExpr {
    fn get_env_inner(&self, scope: &mut Vec<Ident>, env: &mut Vec<Ident>) {
        match &**self {
            hir::Expr::Literal(_) => {},
            hir::Expr::Global(_, _) => {},
            hir::Expr::Intrinsic(_, _, args) => args
                .iter()
                .for_each(|arg| arg.get_env_inner(scope, env)),
            hir::Expr::Local(ident) => {
                if scope.iter().find(|name| *name == ident).is_none()
                    && !env.contains(ident)
                {
                    env.push(*ident);
                }
            },
            hir::Expr::Unary(_, a) => a.get_env_inner(scope, env),
            hir::Expr::Binary(_, a, b) => {
                a.get_env_inner(scope, env);
                b.get_env_inner(scope, env);
            },
            hir::Expr::Tuple(items) => for item in items.iter() {
                item.get_env_inner(scope, env);
            },
            hir::Expr::Record(fields) => for (_, field) in fields.iter() {
                field.get_env_inner(scope, env);
            },
            hir::Expr::List(items) => for item in items.iter() {
                item.get_env_inner(scope, env);
            },
            hir::Expr::Apply(f, arg) => {
                f.get_env_inner(scope, env);
                arg.get_env_inner(scope, env);
            },
            hir::Expr::Access(record, _) => record.get_env_inner(scope, env),
            hir::Expr::Update(record, field, value) => {
                scope.push(**field);
                record.get_env_inner(scope, env);
                value.get_env_inner(scope, env);
                scope.pop();
            },
            hir::Expr::Func(binding, body) => {
                let body_env = body.get_env();
                let bindings = binding.binding_idents();
                for ident in body_env {
                    if scope.iter().find(|name| **name == ident).is_none()
                        && !bindings.contains_key(&ident)
                        && !env.contains(&ident)
                    {
                        env.push(ident);
                    }
                }
            },
            hir::Expr::Match(pred, arms) => {
                pred.get_env_inner(scope, env);
                for (binding, body) in arms.iter() {
                    let mut bindings = binding
                        .binding_idents()
                        .keys()
                        .copied()
                        .collect();
                    let scope_len = scope.len();
                    scope.append(&mut bindings);
                    body.get_env_inner(scope, env);
                    scope.truncate(scope_len);
                }
            },
            hir::Expr::Constructor(_, _, inner) => inner.get_env_inner(scope, env),
        }
    }

    fn get_env(&self) -> Vec<Ident> {
        let mut scope = Vec::new();
        let mut env = Vec::new();
        self.get_env_inner(&mut scope, &mut env);
        env
    }
}

#[derive(Debug)]
pub enum Matcher {
    Wildcard,
    Exactly(Literal),
    Product(Vec<Matcher>),
    List(Vec<Matcher>),
    ListFront(Vec<Matcher>),
}

impl Matcher {
    pub fn is_refutable(&self) -> bool {
        match self {
            Matcher::Wildcard => false,
            Matcher::Exactly(_) => true,
            Matcher::Product(items) => items.iter().any(|item| item.is_refutable()),
            Matcher::List(_) => true,
            Matcher::ListFront(items) => items.len() != 0 // List matches everything
        }
    }
}

// Describes the extraction of pattern bindings from a basic pattern
// For example: ((x, y), _, z)
#[derive(Debug)]
pub enum Extractor {
    Just(Option<Ident>),
    Product(Option<Ident>, Vec<Extractor>),
    List(Option<Ident>, Vec<Extractor>),
    ListFront(Option<Ident>, Vec<Extractor>, Option<Ident>),
}

impl Extractor {
    pub fn extracts_anything(&self) -> bool {
        match self {
            Extractor::Just(x) => x.is_some(),
            Extractor::Product(x, items) => x.is_some() || items.iter().any(|item| item.extracts_anything()),
            Extractor::List(x, items) => x.is_some() || items.iter().any(|item| item.extracts_anything()),
            Extractor::ListFront(x, items, tail) => x.is_some() || tail.is_some() || items.iter().any(|item| item.extracts_anything()),
        }
    }

    fn bindings(&self, bindings: &mut Vec<Ident>) {
        let (this, children, tail) = match self {
            Extractor::Just(x) => (x, None, None),
            Extractor::Product(x, xs) => (x, Some(xs), None),
            Extractor::List(x, xs) => (x, Some(xs), None),
            Extractor::ListFront(x, xs, tail) => (x, Some(xs), *tail),
        };
        if let Some(this) = this {
            bindings.push(*this);
        }
        if let Some(tail) = tail {
            bindings.push(tail);
        }
        for child in children.map(|xs| xs.iter()).into_iter().flatten() {
            child.bindings(bindings);
        }
    }

    pub fn get_bindings(&self) -> Vec<Ident> {
        let mut bindings = Vec::new();
        self.bindings(&mut bindings);
        bindings
    }
}

pub struct Program {
    pub entry: DefId,
    pub globals: HashMap<DefId, Option<RawTypeNode<Expr>>>,
}

impl Program {
    pub fn from_hir(prog: &hir::Program, entry: Ident) -> Result<Self, Error> {
        let mut this = Self {
            entry: LocalIntern::new((entry, Vec::new())),
            globals: HashMap::default(),
        };

        let entry = this.instantiate_def(prog, entry, Vec::new())
            .ok_or_else(|| Error::custom(format!("Cannot find entry point '{}'", *entry)))?;

        Ok(this)
    }

    pub fn globals(&self) -> impl Iterator<Item=(DefId, &RawTypeNode<Expr>)> {
        self.globals.iter().map(|(id, g)| (*id, g.as_ref().unwrap()))
    }

    fn instantiate_def(&mut self, prog: &hir::Program, name: Ident, params: Vec<RawType>) -> Option<DefId> {
        let def_id = LocalIntern::new((name, params.clone()));

        if !self.globals.contains_key(&def_id) {
            self.globals.insert(def_id, None); // Insert phoney to keep recursive functions happy

            let def = prog.root.def(name)?;

            let generics = def.generics
                .iter()
                .zip(params.iter())
                .map(|(name, param)| (**name, param))
                .collect::<HashMap<_, _>>();

            let body = self.instantiate_expr(prog, &def.body, &mut |gen| generics.get(&gen).cloned().cloned().unwrap());
            self.globals.insert(def_id, Some(body));
        }

        Some(def_id)
    }

    fn instantiate_expr(&mut self, prog: &hir::Program, hir_expr: &hir::TypeExpr, get_generic: &mut impl FnMut(Ident) -> RawType) -> RawTypeNode<Expr> {
        let expr = match &**hir_expr {
            hir::Expr::Literal(litr) => Expr::Literal(litr.clone()),
            hir::Expr::Local(local) => Expr::GetLocal(*local),
            hir::Expr::Global(global, generics) => {
                let generics = generics.iter().map(|(_, (_, ty))| self.instantiate_type(prog, ty, get_generic)).collect::<Vec<_>>();
                let def = self.instantiate_def(prog, *global, generics).unwrap();
                Expr::GetGlobal(def)
            },
            hir::Expr::Intrinsic(intrinsic, generics, args) => Expr::Intrinsic(*intrinsic, args
                .iter()
                .map(|arg| self.instantiate_expr(prog, arg, get_generic))
                .collect()),
            hir::Expr::Unary(op, a) => Expr::Unary(**op, self.instantiate_expr(prog, a, get_generic)),
            hir::Expr::Binary(op, a, b) => Expr::Binary(**op, self.instantiate_expr(prog, a, get_generic), self.instantiate_expr(prog, b, get_generic)),
            hir::Expr::Match(pred, arms) => {
                let pred = self.instantiate_expr(prog, pred, get_generic);
                self.instantiate_match(prog, pred, &arms, get_generic)
            },
            hir::Expr::Tuple(items) => Expr::Tuple(items
                .iter()
                .map(|item| self.instantiate_expr(prog, item, get_generic))
                .collect()),
            hir::Expr::Record(fields) => Expr::Tuple(fields
                .iter()
                .map(|(_, field)| self.instantiate_expr(prog, field, get_generic))
                .collect()),
            hir::Expr::List(items) => Expr::List(items
                .iter()
                .map(|item| self.instantiate_expr(prog, item, get_generic))
                .collect()),
            hir::Expr::Func(binding, body) => {
                let extractor = binding.make_extractor(prog);
                let e_bindings = extractor.get_bindings();
                let env = body.get_env().into_iter().filter(|ident| !e_bindings.contains(ident)).collect();
                Expr::Func(extractor, env, self.instantiate_expr(prog, body, get_generic))
            },
            hir::Expr::Apply(f, arg) => Expr::Apply(
                self.instantiate_expr(prog, f, get_generic),
                self.instantiate_expr(prog, arg, get_generic),
            ),
            hir::Expr::Access(record, field) => {
                let fields = match &**record.ty() {
                    Type::Record(fields) => fields,
                    // Proxy types
                    Type::Data(data, params) => match &*prog.data_ctx
                        .get_data(**data)
                        .variants[0].1
                    {
                        Type::Record(fields) => fields,
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                };
                let field_idx = fields
                    .iter()
                    .enumerate().find(|(_, (name, _))| name == field)
                    .unwrap().0;
                Expr::Access(self.instantiate_expr(prog, record, get_generic), field_idx)
            },
            hir::Expr::Update(record, field, value) => match &**record.ty() {
                Type::Record(fields) => {
                    let field_idx = fields
                        .iter()
                        .enumerate().find(|(_, (name, _))| name == field)
                        .unwrap().0;
                    Expr::Update(
                        self.instantiate_expr(prog, record, get_generic),
                        field_idx,
                        **field,
                        self.instantiate_expr(prog, value, get_generic),
                    )
                },
                Type::Data(data, params) => {
                    let fields = match &*prog.data_ctx
                        .get_data(**data)
                        .variants[0].1
                    {
                        Type::Record(fields) => fields,
                        _ => unreachable!(),
                    };
                    let field_idx = fields
                        .iter()
                        .enumerate().find(|(_, (name, _))| name == field)
                        .unwrap().0;
                    Expr::Update(
                        self.instantiate_expr(prog, record, get_generic),
                        field_idx,
                        **field,
                        self.instantiate_expr(prog, value, get_generic),
                    )
                },
                ty => unreachable!("{:?}", ty),
            },
            hir::Expr::Constructor(data, _, inner) => {
                // Sum types with one variant don't need a discriminant!
                let inner = self.instantiate_expr(prog, inner, get_generic);
                if prog.data_ctx.get_data(data.0).variants.len() == 1 {
                    return inner;
                } else {
                    Expr::Tuple(vec![
                        RawTypeNode::new(
                            Expr::Literal(Literal::Number(data.1 as f64)),
                            (data.span(), self.instantiate_type(prog, &Type::Primitive(Primitive::Number), get_generic)),
                        ),
                        inner,
                    ])
                }
            },
        };

        let ty = self.instantiate_type(prog, hir_expr.ty(), get_generic);

        RawTypeNode::new(expr, (hir_expr.span(), ty))
    }

    fn instantiate_match(
        &mut self,
        prog: &hir::Program,
        pred: RawTypeNode<Expr>,
        arms: &[(hir::TypeBinding, hir::TypeExpr)],
        get_generic: &mut impl FnMut(Ident) -> RawType,
    ) -> Expr {
        let arms = arms
            .iter()
            .map(|(binding, body)| (
                binding.make_matcher(prog),
                binding.make_extractor(prog),
                self.instantiate_expr(prog, body, get_generic),
            ))
            .collect();

        Expr::Match(pred, arms)
    }

    fn instantiate_type(&mut self,
        prog: &hir::Program,
        ty: &Type,
        get_generic: &mut dyn FnMut(Ident) -> RawType,
    ) -> RawType {
        match ty {
            Type::Primitive(prim) => RawType::Primitive(prim.clone()),
            Type::GenParam(ident) => get_generic(*ident),
            Type::Tuple(items) => RawType::Product(items
                .iter()
                .map(|item| self.instantiate_type(prog, item, get_generic))
                .collect()),
            Type::Record(fields) => RawType::Product(fields
                .iter()
                .map(|(_, field)| self.instantiate_type(prog, field, get_generic))
                .collect()),
            Type::List(item) => RawType::List(Box::new(self.instantiate_type(prog, item, get_generic))),
            Type::Func(i, o) => RawType::Func(
                Box::new(self.instantiate_type(prog, i, get_generic)),
                Box::new(self.instantiate_type(prog, o, get_generic)),
            ),
            Type::Data(data_id, params) => {
                let data = prog.data_ctx.get_data(**data_id);
                let params = params
                    .iter()
                    .map(|ty| self.instantiate_type(prog, ty, get_generic))
                    .collect::<Vec<_>>();
                let mut get_generic = |name| data.generics
                    .iter()
                    .zip(params.iter())
                    .find(|(gen, _)| ***gen == name)
                    .map(|(_, ty)| ty.clone())
                    .unwrap();
                // Sum types with one variant don't need a discriminant!
                if data.variants.len() == 1 {
                    //self.instantiate_type(prog, &data.variants[0], &mut get_generic)
                    RawType::Boxed(
                        prog.data_ctx.get_data_name(**data_id),
                        params,
                    )
                } else {
                    RawType::Sum(data.variants
                        .iter()
                        .map(|(variant, ty)| RawType::Boxed(
                            Ident::new(format!("{}::{}", prog.data_ctx.get_data_name(**data_id), **variant)),
                            params.clone(),
                        )) // self.instantiate_type(prog, ty, &mut get_generic)
                        .collect())
                }
            },
        }
    }
}
