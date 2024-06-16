import SwiftUI

struct Cove: View {
    @State var rust: ViewModel;
    
    var body: some View {
        HStack {
            Button(action: {
                self.rust.dispatch(event: .decrement)
            }) {
                Text("-")
                    .font(.largeTitle)
                    .frame(width: 50, height: 50)
                    .background(Color.red)
                    .foregroundColor(.white)
                    .cornerRadius(10)
            }

            Text("\(self.rust.count)")
                .font(.largeTitle)
                .frame(width: 50, height: 50)

            Button(action: {
                self.rust.dispatch(event: .increment)
            }) {
                Text("+")
                    .font(.largeTitle)
                    .frame(width: 50, height: 50)
                    .background(Color.green)
                    .foregroundColor(.white)
                    .cornerRadius(10)
            }
        }
        .padding()
    }
}
