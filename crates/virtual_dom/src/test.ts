interface IBase {
    id: number
}


class Base implements IBase{
    id = 0
}

interface IPersona {
    nombre: string
    apellido: string
    trabajo: string|null
}
class Persona implements IBase, IPersona{
    id: 0;
    nombre = "José"
    apellido = "Vargas"
    trabajo = null
}

interface IMedico {
    trabajo : "Medico"
    especializacion: string
}
class Medico implements IBase, IPersona, IMedico{
    id: 0;
    nombre = "José"
    apellido = "Vargas"
    trabajo = "Medico" as const
    especializacion = "Cirujia"
}

interface IConductor {
    trabajo : "conductor"
    tipoDeVeiculo: string
}
class Conductor implements IBase, IPersona, IConductor{
    id: 0;
    nombre = "José"
    apellido = "Vargas"
    trabajo = "conductor" as const
    tipoDeVeiculo = "camion"
}

const list = [new Persona(), new Persona(), new Medico(), new Conductor()]

console.log(list[5].id)

if(list[5] instanceof Persona) console.log(list[5].nombre)
if(list[5] instanceof Medico) console.log(list[5].especializacion)
if(list[5] instanceof Conductor) console.log(list[5].tipoDeVeiculo)