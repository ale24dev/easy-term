import "./App.css";

function App() {
  return (
    <main className="popover">
      <header className="popover-header">
        <span className="popover-title">easy-term</span>
      </header>
      <div className="empty-state">
        <p>No hay proyectos todavía.</p>
        <p className="hint">Agrega uno para empezar.</p>
      </div>
    </main>
  );
}

export default App;
